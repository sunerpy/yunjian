use std::path::PathBuf;

/// 语音路径的失败面。每一项都对应一个可降级的场景：调用方永远可以退回默写练习，
/// 而不是把整个应用带下去。
#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error(
        "本二进制未编译语音能力：需要 `--features voice` 重新构建，详见 docs/VOICE-BUILD.zh.md"
    )]
    FeatureDisabled,

    #[error(
        "模型文件缺失：{}；模型按需下载由后续任务实现，当前需手动放置，详见 docs/VOICE-BUILD.zh.md",
        path.display()
    )]
    ModelMissing { path: PathBuf },

    #[error("音频文件读取失败：{}：{source}", path.display())]
    AudioRead {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("音频文件写入失败：{}：{source}", path.display())]
    AudioWrite {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("sherpa-onnx 调用失败：{0}")]
    Backend(String),

    #[error("没有可用的麦克风：{detail}")]
    NoInputDevice {
        /// 具体是「系统没报告默认设备」还是「指定的名字匹配不到」。
        detail: String,
    },

    #[error("音频设备操作失败：{detail}：{source}")]
    AudioDevice {
        /// 失败发生在哪一步（枚举、配置、打开流）。
        detail: String,
        /// 底层错误。
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// 设备既不产出样本也不报错。`Microphone` 的迭代器在这种情形下永久阻塞，
    /// 所以必须由外层的 wall-clock 超时判定，而不是等它自己失败。
    #[error("麦克风在 {waited:?} 内没有产出任何样本，判定为设备停摆")]
    CaptureStalled {
        /// 实际等待时长。
        waited: std::time::Duration,
    },
}

impl VoiceError {
    /// 该错误对应的降级原因。**每一条都必须有**——这是「任何失败路径都退回打字练习」
    /// 由类型系统保证而非靠约定的地方。
    #[must_use]
    pub const fn degrade_reason(&self) -> crate::permission::DegradeReason {
        use crate::permission::DegradeReason as R;
        match self {
            Self::FeatureDisabled => R::FeatureDisabled,
            Self::NoInputDevice { .. } => R::NoInputDevice,
            Self::ModelMissing { .. }
            | Self::AudioRead { .. }
            | Self::AudioWrite { .. }
            | Self::Backend(_)
            | Self::AudioDevice { .. }
            | Self::CaptureStalled { .. } => R::CaptureFailed,
        }
    }
}

impl VoiceError {
    /// 目录必须存在且包含全部指定文件，否则报出**第一个**缺失项的完整路径。
    /// 报目录名不够用：调用方拿到的报错要能直接 `ls`。
    ///
    /// `cfg` 里带 `test` 是为了让不开 `voice` 的构建既没有死代码告警，又仍然跑到
    /// 这条错误路径的单元测试。
    #[cfg(any(feature = "voice", test))]
    pub(crate) fn require_files(dir: &std::path::Path, files: &[&str]) -> Result<(), Self> {
        for name in files {
            let path = dir.join(name);
            if !path.is_file() {
                return Err(Self::ModelMissing { path });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::VoiceError;

    #[test]
    fn require_files_names_the_first_missing_path() {
        let dir = std::env::temp_dir().join("yunjian-voice-error-test");
        std::fs::create_dir_all(&dir).expect("临时目录可创建");
        let err = VoiceError::require_files(&dir, &["nope.onnx"]).expect_err("应报缺失");
        let text = err.to_string();
        assert!(text.contains("nope.onnx"), "报错要给出完整路径：{text}");
        assert!(text.contains("VOICE-BUILD"), "报错要指向构建文档：{text}");
    }

    #[test]
    fn feature_disabled_message_names_the_flag() {
        let text = VoiceError::FeatureDisabled.to_string();
        assert!(text.contains("--features voice"), "{text}");
    }
}
