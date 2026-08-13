//! 经 `sherpa_rs_sys` 的在线 API 实现的双路流式解码。
//!
//! `sherpa-rs` 0.6.8 只封装了 offline 识别器（`whisper.rs` / `zipformer.rs` 走的都是
//! `SherpaOnnxOfflineRecognizer`），流式必须直调底层符号。`crate::backend_version`
//! 已有直调先例，这里沿用同一形态。
//!
//! # 一个识别器、两条流
//!
//! `SherpaOnnxCreateOnlineStreamWithHotwords` 让 hotwords 成为**流级**而非识别器级的设置，
//! 于是两路解码可以共用一份权重：无偏置流走 `SherpaOnnxCreateOnlineStream`，偏置流走带
//! hotwords 的那个。这不只是省内存——两条流跑在同一份权重上，双路成本的差值就纯粹是
//! 解码开销，而不掺入两次模型加载的差异。
//!
//! # 为什么模型族是 Transducer 而不是 Whisper
//!
//! Whisper 是 encoder-decoder，没有流式接口。`models.toml` 里唯一许可通过的流式模型是
//! `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20`（Apache-2.0），
//! 因此流式路径固定在 Transducer 上。

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sherpa_rs::sherpa_rs_sys as sys;

use crate::VoiceError;
use crate::recognize::{
    DualDecode, DualDecodeCost, OnlineDecodeConfig, PartialHypothesis, Rtf, UnbiasedAsrHyp,
    biased_hyp,
};

/// 流式 Transducer 的四个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransducerFiles {
    /// 编码器。
    pub encoder: PathBuf,
    /// 解码器。
    pub decoder: PathBuf,
    /// joiner。
    pub joiner: PathBuf,
    /// token 表。
    pub tokens: PathBuf,
}

impl TransducerFiles {
    /// 在模型目录里按 sherpa-onnx 的发布布局定位四个文件。
    ///
    /// 精度后缀由目录内容推断而不是写死：官方包里 `encoder-epoch-99-avg-1.onnx` 与
    /// `.int8.onnx` 并存，写死任何一个都会在换包时报「模型缺失」。
    ///
    /// # Errors
    ///
    /// `ModelMissing` 指出第一个缺失项；`Backend` 说明目录不像流式 Transducer 包。
    pub fn discover(model_dir: &Path, int8: bool) -> Result<Self, VoiceError> {
        if !model_dir.is_dir() {
            return Err(VoiceError::ModelMissing {
                path: model_dir.to_path_buf(),
            });
        }
        let suffix = if int8 { ".int8.onnx" } else { ".onnx" };
        let pick = |kind: &str| -> Option<PathBuf> {
            let mut found: Vec<PathBuf> = std::fs::read_dir(model_dir)
                .ok()?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    name.starts_with(kind) && name.ends_with(suffix)
                })
                .collect();
            found.sort();
            found.pop()
        };
        let encoder =
            pick("encoder").ok_or_else(|| missing_layout(model_dir, "encoder", suffix))?;
        let decoder =
            pick("decoder").ok_or_else(|| missing_layout(model_dir, "decoder", suffix))?;
        let joiner = pick("joiner").ok_or_else(|| missing_layout(model_dir, "joiner", suffix))?;
        let tokens = model_dir.join("tokens.txt");
        if !tokens.is_file() {
            return Err(VoiceError::ModelMissing { path: tokens });
        }
        Ok(Self {
            encoder,
            decoder,
            joiner,
            tokens,
        })
    }
}

fn missing_layout(model_dir: &Path, kind: &str, suffix: &str) -> VoiceError {
    VoiceError::Backend(format!(
        "{} 里没有 `{kind}*{suffix}`，不像 sherpa-onnx 的流式 Transducer 模型目录",
        model_dir.display()
    ))
}

/// 一路解码的持有物：识别器、流，以及它自己那份累计墙钟时间。
struct Pass {
    recognizer: *const sys::SherpaOnnxOnlineRecognizer,
    stream: *const sys::SherpaOnnxOnlineStream,
    decode_wall: Duration,
    /// 传给 C 的字符串必须活到识别器销毁：`SherpaOnnxCreateOnlineRecognizer` 只借指针。
    _owned: Vec<CString>,
}

impl Drop for Pass {
    #[expect(
        unsafe_code,
        reason = "释放 C API 返回的句柄，两个指针都由本结构独占持有"
    )]
    fn drop(&mut self) {
        // SAFETY: 两个指针都由 `Pass::open` 从对应的 create 函数取得，未被别处释放，
        // 且流必须先于识别器销毁。
        unsafe {
            sys::SherpaOnnxDestroyOnlineStream(self.stream);
            sys::SherpaOnnxDestroyOnlineRecognizer(self.recognizer);
        }
    }
}

impl Pass {
    #[expect(
        unsafe_code,
        reason = "sherpa-rs 0.6.8 没有流式识别器封装，只能直调 C API"
    )]
    fn open(files: &TransducerFiles, config: &OnlineDecodeConfig) -> Result<Self, VoiceError> {
        config.validate()?;
        let mut owned = Vec::new();
        let mut keep = |text: String| -> Result<*const std::os::raw::c_char, VoiceError> {
            let value = CString::new(text).map_err(|e| VoiceError::RecognizerConfig {
                detail: format!("配置里出现了 NUL 字节：{e}"),
            })?;
            let pointer = value.as_ptr();
            owned.push(value);
            Ok(pointer)
        };

        let hotwords_buf = match config.hotwords.as_ref() {
            Some(hotwords) => keep(hotwords.buffer())?,
            None => std::ptr::null(),
        };
        let hotwords_len = config
            .hotwords
            .as_ref()
            .map_or(0, |hotwords| hotwords.buffer().len());
        let hr = match config.homophone_replacer.as_ref() {
            Some(replacer) => sys::SherpaOnnxHomophoneReplacerConfig {
                dict_dir: keep(replacer.dict_dir.to_string_lossy().into_owned())?,
                lexicon: keep(replacer.lexicon.to_string_lossy().into_owned())?,
                rule_fsts: keep(replacer.rule_fsts.to_string_lossy().into_owned())?,
            },
            None => sys::SherpaOnnxHomophoneReplacerConfig {
                dict_dir: std::ptr::null(),
                lexicon: std::ptr::null(),
                rule_fsts: std::ptr::null(),
            },
        };

        let model_config = sys::SherpaOnnxOnlineModelConfig {
            transducer: sys::SherpaOnnxOnlineTransducerModelConfig {
                encoder: keep(files.encoder.to_string_lossy().into_owned())?,
                decoder: keep(files.decoder.to_string_lossy().into_owned())?,
                joiner: keep(files.joiner.to_string_lossy().into_owned())?,
            },
            tokens: keep(files.tokens.to_string_lossy().into_owned())?,
            num_threads: config.num_threads,
            provider: keep("cpu".to_owned())?,
            // 刻意留空让 sherpa-onnx 从 ONNX 元数据里读模型类型。写死任何一个值都会在换包时
            // 报「`query_head_dims` does not exist in the metadata」这类难以归因的错误——
            // 白名单里那个 2023-02-20 双语包是 zipformer 而非 zipformer2，实测正是这么失败的。
            model_type: keep(String::new())?,
            modeling_unit: keep(config.modeling_unit.as_str().to_owned())?,
            ..unsafe { std::mem::zeroed() }
        };

        let raw = sys::SherpaOnnxOnlineRecognizerConfig {
            feat_config: sys::SherpaOnnxFeatureConfig {
                sample_rate: 16_000,
                feature_dim: 80,
            },
            model_config,
            decoding_method: keep(config.decoding_method.as_str().to_owned())?,
            max_active_paths: config.max_active_paths,
            enable_endpoint: 1,
            rule1_min_trailing_silence: config.endpoint.min_trailing_silence_before_speech,
            rule2_min_trailing_silence: config.endpoint.min_trailing_silence_after_speech,
            rule3_min_utterance_length: config.endpoint.max_utterance_length,
            hotwords_score: config.hotwords_score,
            // ITN 的入口就是 `rule_fsts`。它从 `config.rule_fsts()` 取值，而那个方法恒为
            // 空串且没有任何字段能改它——这是「ITN 保持关闭」在生产路径上的落点。
            rule_fsts: keep(config.rule_fsts().to_owned())?,
            hotwords_buf,
            hotwords_buf_size: i32::try_from(hotwords_len).unwrap_or(i32::MAX),
            hr,
            ..unsafe { std::mem::zeroed() }
        };

        // SAFETY: `raw` 里的每个指针都指向 `owned` 持有的 CString 或空指针，且 `owned`
        // 随 `Pass` 一起存活到识别器销毁之后。
        let recognizer = unsafe { sys::SherpaOnnxCreateOnlineRecognizer(&raw) };
        if recognizer.is_null() {
            return Err(VoiceError::Backend(
                "SherpaOnnxCreateOnlineRecognizer 返回空指针".to_owned(),
            ));
        }

        // SAFETY: `recognizer` 刚由上一步创建且非空；带 hotwords 时缓冲区仍由 `owned` 持有。
        let stream = unsafe {
            if hotwords_buf.is_null() {
                sys::SherpaOnnxCreateOnlineStream(recognizer)
            } else {
                sys::SherpaOnnxCreateOnlineStreamWithHotwords(recognizer, hotwords_buf)
            }
        };
        if stream.is_null() {
            // SAFETY: 流创建失败，识别器仍需释放，且此时没有任何流引用它。
            unsafe { sys::SherpaOnnxDestroyOnlineRecognizer(recognizer) };
            return Err(VoiceError::Backend(
                "SherpaOnnxCreateOnlineStream 返回空指针".to_owned(),
            ));
        }

        Ok(Self {
            recognizer,
            stream,
            decode_wall: Duration::ZERO,
            _owned: owned,
        })
    }

    #[expect(unsafe_code, reason = "直调 C API 的解码循环")]
    fn accept(&mut self, samples: &[f32], sample_rate: u32) {
        let length = i32::try_from(samples.len()).unwrap_or(i32::MAX);
        let rate = i32::try_from(sample_rate).unwrap_or(16_000);
        let started = Instant::now();
        // SAFETY: 切片在调用期间有效，长度已收窄到 i32；`AcceptWaveform` 只读取它。
        unsafe {
            sys::SherpaOnnxOnlineStreamAcceptWaveform(self.stream, rate, samples.as_ptr(), length);
            while sys::SherpaOnnxIsOnlineStreamReady(self.recognizer, self.stream) == 1 {
                sys::SherpaOnnxDecodeOnlineStream(self.recognizer, self.stream);
            }
        }
        self.decode_wall += started.elapsed();
    }

    #[expect(unsafe_code, reason = "直调 C API 的收尾解码")]
    fn finish(&mut self) {
        let started = Instant::now();
        // SAFETY: 两个指针都仍然有效；`InputFinished` 之后只能解码不能再喂数据。
        unsafe {
            sys::SherpaOnnxOnlineStreamInputFinished(self.stream);
            while sys::SherpaOnnxIsOnlineStreamReady(self.recognizer, self.stream) == 1 {
                sys::SherpaOnnxDecodeOnlineStream(self.recognizer, self.stream);
            }
        }
        self.decode_wall += started.elapsed();
    }

    #[expect(unsafe_code, reason = "读取并释放 C API 返回的结果结构")]
    fn text(&self) -> String {
        // SAFETY: `GetOnlineStreamResult` 返回需由调用方释放的结构；`text` 是 NUL 结尾
        // 的 C 字符串，且在释放前有效。
        unsafe {
            let result = sys::SherpaOnnxGetOnlineStreamResult(self.recognizer, self.stream);
            if result.is_null() {
                return String::new();
            }
            let text = (*result).text;
            let owned = if text.is_null() {
                String::new()
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            };
            sys::SherpaOnnxDestroyOnlineRecognizerResult(result);
            owned
        }
    }
}

/// 双路流式解码器。
///
/// 两路共用一份权重（见模块文档），偏置那一路可缺席——[`crate::recognize::DecodePlan`]
/// 判定双路跑不动时就不开它，而不是丢帧。
pub struct StreamingDualDecoder {
    unbiased: Pass,
    unbiased_config: OnlineDecodeConfig,
    biased: Option<Pass>,
    biased_config: Option<OnlineDecodeConfig>,
    audio: Duration,
    at_ms: u64,
}

// SAFETY: 断言的是**独占所有权可以移动到另一个线程**，不是可以并发共享——所以只实现
// `Send` 而刻意不实现 `Sync`。两个句柄都由 `Pass` 独占持有，除 `Drop` 外没有别的持有者；
// sherpa-onnx 的 online recognizer 与 online stream 不带线程局部状态，其官方示例也在
// 工作线程里持有它们。[`crate::recognize::start_recognition`] 恰好只需要这一条：把解码器
// move 进那唯一的工作线程，全程只有那个线程访问它。
#[expect(
    unsafe_code,
    reason = "为把解码器 move 进长操作的工作线程而断言独占所有权可跨线程转移"
)]
unsafe impl Send for StreamingDualDecoder {}

impl StreamingDualDecoder {
    /// 打开双路解码器。
    ///
    /// `biased` 为 `None` 时只开无偏置一路，这正是超出实时预算后的降级形态。
    ///
    /// # Errors
    ///
    /// 转述模型定位与识别器构造的失败；配置不自洽时报
    /// [`VoiceError::RecognizerConfig`]。
    pub fn open(
        files: &TransducerFiles,
        unbiased: OnlineDecodeConfig,
        biased: Option<OnlineDecodeConfig>,
    ) -> Result<Self, VoiceError> {
        if unbiased.is_biased() {
            return Err(VoiceError::RecognizerConfig {
                detail: "无偏置一路不得带 hotwords，否则它签出的见证是假的".to_owned(),
            });
        }
        if let Some(config) = biased.as_ref()
            && !config.is_biased()
        {
            return Err(VoiceError::RecognizerConfig {
                detail: "偏置一路必须带 hotwords，否则高亮没有对齐依据".to_owned(),
            });
        }
        let unbiased_pass = Pass::open(files, &unbiased)?;
        let biased_pass = match biased.as_ref() {
            Some(config) => Some(Pass::open(files, config)?),
            None => None,
        };
        Ok(Self {
            unbiased: unbiased_pass,
            unbiased_config: unbiased,
            biased: biased_pass,
            biased_config: biased,
            audio: Duration::ZERO,
            at_ms: 0,
        })
    }

    fn snapshot(&self) -> PartialHypothesis {
        PartialHypothesis {
            at_ms: self.at_ms,
            unbiased: UnbiasedAsrHyp::from_pass(&self.unbiased_config, self.unbiased.text()),
            biased: match (self.biased.as_ref(), self.biased_config.as_ref()) {
                (Some(pass), Some(config)) => biased_hyp(config, pass.text()),
                _ => None,
            },
        }
    }
}

impl DualDecode for StreamingDualDecoder {
    fn accept(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<PartialHypothesis, VoiceError> {
        if sample_rate == 0 {
            return Err(VoiceError::RecognizerConfig {
                detail: "采样率为 0".to_owned(),
            });
        }
        self.audio += Duration::from_secs_f64(samples.len() as f64 / f64::from(sample_rate));
        self.at_ms = u64::try_from(self.audio.as_millis()).unwrap_or(u64::MAX);
        self.unbiased.accept(samples, sample_rate);
        if let Some(pass) = self.biased.as_mut() {
            pass.accept(samples, sample_rate);
        }
        Ok(self.snapshot())
    }

    fn finish(&mut self) -> Result<PartialHypothesis, VoiceError> {
        self.unbiased.finish();
        if let Some(pass) = self.biased.as_mut() {
            pass.finish();
        }
        Ok(self.snapshot())
    }

    fn cost(&self) -> DualDecodeCost {
        let single = self.unbiased.decode_wall;
        let biased = self
            .biased
            .as_ref()
            .map_or(Duration::ZERO, |pass| pass.decode_wall);
        DualDecodeCost {
            single: Rtf::measure(self.audio, single),
            dual: Rtf::measure(self.audio, single + biased),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TransducerFiles;

    #[test]
    fn discovery_reports_a_non_transducer_directory() {
        let dir =
            std::env::temp_dir().join(format!("yunjian-streaming-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");

        let error = TransducerFiles::discover(&dir, false).expect_err("空目录应报错");
        assert!(error.to_string().contains("encoder"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// int8 与 fp32 并存时按请求的精度各自挑到自己那一组，而不是把 int8 当成 fp32。
    #[test]
    fn discovery_separates_int8_from_float_weights() {
        let dir =
            std::env::temp_dir().join(format!("yunjian-streaming-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        for name in [
            "encoder-epoch-99-avg-1.onnx",
            "encoder-epoch-99-avg-1.int8.onnx",
            "decoder-epoch-99-avg-1.onnx",
            "decoder-epoch-99-avg-1.int8.onnx",
            "joiner-epoch-99-avg-1.onnx",
            "joiner-epoch-99-avg-1.int8.onnx",
            "tokens.txt",
        ] {
            std::fs::write(dir.join(name), b"x").expect("可写入");
        }

        let float = TransducerFiles::discover(&dir, false).expect("fp32 应定位成功");
        assert!(
            float
                .encoder
                .to_string_lossy()
                .ends_with("encoder-epoch-99-avg-1.onnx")
        );
        let int8 = TransducerFiles::discover(&dir, true).expect("int8 应定位成功");
        assert!(int8.joiner.to_string_lossy().ends_with(".int8.onnx"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
