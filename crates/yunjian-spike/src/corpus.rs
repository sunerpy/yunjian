//! 判据②：走生产下载路径把发布语料物化进应用私有存储，并只对**判据声明的窗口**计时。
//!
//! # 为什么是「观察生产路径」而不是「复刻生产路径」
//!
//! 判据②的措辞是「必须是生产使用的同一条路径」。生产入口是
//! [`AssetResolver::sync`]：它下载 `.db.gz`、按清单里的 SHA-256 校验、把归档原子发布，
//! 再由 [`CorpusHandle::open`] 校验一次归档摘要、解压并原子落盘。本模块**原样调用它**，
//! 一个字节都不重写，然后从旁观察它在文件系统上留下的里程碑来分段计时。
//!
//! 之所以要旁观而不是接进度回调：`AssetResolver::sync` 不接受 `progress`（它内部调用的是
//! 无回调的 `CorpusHandle::open`）。为了拿到分段耗时而给生产函数加一个仅测试需要的参数，
//! 等于为了量它而改它。旁观的代价只有一个采样周期的误差，而阈值是 60 秒——把
//! `poll_interval_ms` 一并记进报告，读者就能自己判断这点误差是否要紧。
//!
//! # 为什么在落盘处停表，而不等首启派生
//!
//! 判据②的窗口是「下载 + 校验 + 原子解压」。首启派生是另一件事，唐宋规模实测 571.8 s，
//! 把它算进来会让这条判据必然超阈值，而方案从未把派生写进阈值。所以观察到语料落盘即
//! 停表并返回；派生线程被**刻意遗弃**，由 instrumentation 进程退出时一并回收。
//! 这件事记在 `derive_awaited` 里，不藏着。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest as _, Sha256};
use yunjian_core::assets::AssetResolver;
use yunjian_core::{CORPUS_ARCHIVE_NAME, CORPUS_FILE_NAME, CorpusConfig};

/// 观察生产路径时的采样周期。
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// 观察生产路径的总预算。**远大于判据的 60 秒阈值**：预算的作用是不让测试永远挂着，
/// 不是替阈值下结论。超时同样如实上报，由 `xtask` 判成超阈值。
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(600);

/// 判据②在应用私有存储里使用的子目录。
const DATA_SUBDIR: &str = "corpus";

/// 生产路径在文件系统上的三个里程碑。
#[derive(Debug, Clone)]
pub struct Layout {
    /// 语料落地目录，即 `CorpusConfig::data_dir`。
    pub data_dir: PathBuf,
    /// 已发布的下载归档。
    pub archive: PathBuf,
    /// 归档旁的 `.sha256`；生产路径**只在校验通过后**才写它。
    pub sidecar: PathBuf,
    /// 原子落盘后的语料库。
    pub corpus: PathBuf,
}

impl Layout {
    /// 按生产命名约定推导三个里程碑的位置。
    #[must_use]
    pub fn new(data_root: &Path) -> Self {
        let data_dir = data_root.join(DATA_SUBDIR);
        let archive = data_dir.join(CORPUS_ARCHIVE_NAME);
        let mut sidecar = archive.clone().into_os_string();
        sidecar.push(".sha256");
        Self {
            corpus: data_dir.join(CORPUS_FILE_NAME),
            sidecar: PathBuf::from(sidecar),
            archive,
            data_dir,
        }
    }
}

/// 旁观所得的分段耗时。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observation {
    /// 从起表到归档旁的 `.sha256` 出现。生产路径只在摘要核对通过后写它，
    /// 因此这个时刻同时意味着「下载完成」与「校验通过」。
    pub download_verified: Option<Duration>,
    /// 从起表到语料库出现在生产位置。
    pub materialized: Option<Duration>,
    /// 生产路径提前失败时它自己给出的原因。
    pub worker_error: Option<String>,
    /// 预算耗尽而两个里程碑都没齐。
    pub timed_out: bool,
}

/// 旁观生产路径，直到语料落盘、生产路径报错，或预算耗尽。
///
/// 抽成独立函数是为了能在宿主上真跑：测试用一个按已知时序 `touch` 文件的假线程替换
/// 生产路径，于是「里程碑判定」这段逻辑不必靠一台真机才能被验证一次。
pub fn observe(
    layout: &Layout,
    done: &Receiver<Result<(), String>>,
    budget: Duration,
    tick: Duration,
) -> Observation {
    let started = Instant::now();
    let mut seen = Observation::default();
    loop {
        if seen.download_verified.is_none() && layout.sidecar.is_file() {
            seen.download_verified = Some(started.elapsed());
        }
        if layout.corpus.is_file() {
            seen.materialized = Some(started.elapsed());
            return seen;
        }
        match done.recv_timeout(tick) {
            // 生产路径已经返回。**仍要再看一眼文件系统**：落盘与线程返回之间有一个采样
            // 间隔的窗口，直接退出会把一次成功的物化误报成「里程碑没出现」。
            Ok(outcome) => {
                if layout.sidecar.is_file() && seen.download_verified.is_none() {
                    seen.download_verified = Some(started.elapsed());
                }
                if layout.corpus.is_file() {
                    seen.materialized = Some(started.elapsed());
                }
                if let Err(reason) = outcome {
                    seen.worker_error = Some(reason);
                }
                return seen;
            }
            Err(RecvTimeoutError::Disconnected) => {
                if layout.corpus.is_file() {
                    seen.materialized = Some(started.elapsed());
                } else {
                    seen.worker_error =
                        Some("生产路径线程在未落盘也未回报结果的情况下消失".to_owned());
                }
                return seen;
            }
            Err(RecvTimeoutError::Timeout) => {
                if started.elapsed() >= budget {
                    seen.timed_out = true;
                    return seen;
                }
            }
        }
    }
}

/// 判据②交给宿主侧的全部测量值。字段名与 `xtask` 的 `required_measurements` 对齐。
#[derive(Debug, Clone, Serialize)]
pub struct Measurement {
    /// 实际使用的清单来源。空字符串意味着走了生产默认发布地址。
    pub manifest_url: String,
    /// 被调用的生产符号，写进报告供人核对「这真是生产路径」。
    pub production_path: &'static str,
    /// 下载归档的字节数。
    pub artifact_bytes: Option<u64>,
    /// 生产路径的 SHA-256 校验是否通过。
    pub sha256_verified: bool,
    /// 归档实际摘要，与 `.sha256` 旁文件逐字比对过。
    pub sha256: Option<String>,
    /// 判据②的窗口耗时：下载 + 校验 + 原子解压。
    pub duration_seconds: Option<f64>,
    /// 窗口内「下载 + 校验」的分段耗时。
    pub download_verify_seconds: Option<f64>,
    /// 窗口内「原子解压落盘」的分段耗时。
    pub decompress_seconds: Option<f64>,
    /// 落盘后语料库的字节数。
    pub corpus_bytes: Option<u64>,
    /// 是否原子落盘：语料在生产位置就绪且目录里没有残留临时文件。
    pub atomic_install: bool,
    /// 生产路径是否在窗口内崩溃或报错。
    pub crashed: bool,
    /// 观察周期，让读者能判断分段耗时的精度。
    pub poll_interval_ms: u64,
    /// 是否等过首启派生。恒为 `false`，且必须写出来。
    pub derive_awaited: bool,
    /// 起表前是否清空过落地目录，即这是否是一次真的首启。
    pub started_from_clean_state: bool,
    /// 目录里残留的临时文件数量；原子性判定的依据。
    pub residual_temp_files: usize,
    /// 生产路径给出的失败原因；成功时为 `None`。
    pub failure: Option<String>,
}

impl Measurement {
    fn failed(manifest_url: &str, reason: String) -> Self {
        Self {
            manifest_url: manifest_url.to_owned(),
            production_path: PRODUCTION_PATH,
            artifact_bytes: None,
            sha256_verified: false,
            sha256: None,
            duration_seconds: None,
            download_verify_seconds: None,
            decompress_seconds: None,
            corpus_bytes: None,
            atomic_install: false,
            crashed: true,
            poll_interval_ms: POLL_INTERVAL.as_millis() as u64,
            derive_awaited: false,
            started_from_clean_state: false,
            residual_temp_files: 0,
            failure: Some(reason),
        }
    }

    /// 序列化成 Kotlin 侧要转发的 JSON。序列化不该失败，真失败时也要给出可读的东西。
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| {
            format!("{{\"crashed\":true,\"failure\":\"测量序列化失败：{error}\"}}")
        })
    }
}

/// 被调用的生产符号。写成常量而不是散在字符串里，改动生产入口时这里会一起被搜到。
const PRODUCTION_PATH: &str = "yunjian_core::assets::AssetResolver::{discover,new}+sync";

/// 走生产路径物化语料并返回判据②的测量值。
///
/// `manifest_url` 为空时使用 [`AssetResolver::discover`]，也就是产品自己的默认发布地址；
/// 非空时使用 [`AssetResolver::new`]，供本机以 `file://` 复现同一条路径。
#[must_use]
pub fn measure(manifest_url: &str, data_root: &Path, budget: Duration) -> Measurement {
    let layout = Layout::new(data_root);
    // 判据②量的是**首启**。上一轮留下的归档或语料会让生产路径走「已存在」的分支，
    // 于是量到的是一次零耗时的空转。清目录不是为了好看，是为了让这次真的是首启。
    let cleaned = match reset(&layout) {
        Ok(()) => true,
        Err(error) => {
            return Measurement::failed(manifest_url, format!("清理落地目录失败：{error}"));
        }
    };

    let corpus_config = CorpusConfig {
        path: None,
        data_dir: layout.data_dir.clone(),
        archive: None,
    };
    let resolver = if manifest_url.trim().is_empty() {
        AssetResolver::discover(corpus_config, data_root)
    } else {
        AssetResolver::new(manifest_url.to_owned(), corpus_config, data_root)
    };

    let (tx, rx) = channel();
    let worker = std::thread::Builder::new()
        .name("yunjian-spike-corpus".to_owned())
        // 生产路径解压 633 MiB 并在同一栈上进入首启派生；默认 2 MiB 栈在真机上不够用过一次，
        // 这里显式给足，避免把栈溢出误读成「语料物化崩溃」。
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let outcome = resolver
                .sync(|_asset, _seed, _corpus| Ok(()))
                .map(|_| ())
                .map_err(|error| error.to_string());
            // 发送失败只意味着旁观者已经收表走人，这是预期路径而不是错误。
            let _ = tx.send(outcome);
        });
    if let Err(error) = worker {
        return Measurement::failed(manifest_url, format!("无法启动生产路径线程：{error}"));
    }

    let seen = observe(&layout, &rx, budget, POLL_INTERVAL);
    let residual = residual_temp_files(&layout.data_dir);
    let archive_bytes = file_len(&layout.archive);
    let corpus_bytes = file_len(&layout.corpus);
    let (sha256, sha256_verified) = verify_recorded_digest(&layout);

    let failure = seen.worker_error.clone().or_else(|| {
        if seen.timed_out {
            Some(format!(
                "旁观预算 {} 秒耗尽，生产路径仍未把语料落到 {}",
                budget.as_secs(),
                layout.corpus.display()
            ))
        } else if seen.materialized.is_none() {
            Some("生产路径返回但语料未出现在生产位置".to_owned())
        } else {
            None
        }
    });

    Measurement {
        manifest_url: manifest_url.to_owned(),
        production_path: PRODUCTION_PATH,
        artifact_bytes: archive_bytes,
        sha256_verified,
        sha256,
        duration_seconds: seen.materialized.map(duration_seconds),
        download_verify_seconds: seen.download_verified.map(duration_seconds),
        decompress_seconds: match (seen.download_verified, seen.materialized) {
            (Some(download), Some(total)) => total.checked_sub(download).map(duration_seconds),
            _ => None,
        },
        corpus_bytes,
        atomic_install: seen.materialized.is_some() && residual == 0,
        // 判据②的 `crashed` 问的是「窗口内产品是否挂了」。旁观者能看见的形态有两种：
        // 生产路径回报了 Err，或者语料始终没出现。两者都算，不到达即不算。
        crashed: failure.is_some(),
        poll_interval_ms: POLL_INTERVAL.as_millis() as u64,
        derive_awaited: false,
        started_from_clean_state: cleaned,
        residual_temp_files: residual,
        failure,
    }
}

fn duration_seconds(value: Duration) -> f64 {
    // 保留三位小数：判据阈值是整秒，再多的位数只会让报告难读。
    (value.as_secs_f64() * 1000.0).round() / 1000.0
}

fn reset(layout: &Layout) -> std::io::Result<()> {
    if layout.data_dir.exists() {
        std::fs::remove_dir_all(&layout.data_dir)?;
    }
    std::fs::create_dir_all(&layout.data_dir)
}

fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

/// 统计落地目录里残留的临时文件。生产路径的临时名固定以 `.tmp` 结尾
/// （`assets::temp_path`），原子发布成功后不该留下任何一个。
fn residual_temp_files(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".tmp"))
        })
        .count()
}

/// 重算归档摘要并与生产路径写下的 `.sha256` 逐字比对。
///
/// 生产路径只在校验通过后才写这个旁文件，所以它存在本身已是「校验通过」的证据；
/// 这里再算一遍是为了让报告里的 `sha256` 是**本次实测**的值，而不是转抄一份声明。
fn verify_recorded_digest(layout: &Layout) -> (Option<String>, bool) {
    let Ok(recorded) = std::fs::read_to_string(&layout.sidecar) else {
        return (None, false);
    };
    let Some(expected) = recorded.split_whitespace().next() else {
        return (None, false);
    };
    let Some(actual) = sha256_of(&layout.archive) else {
        return (None, false);
    };
    let matched = actual == expected.to_ascii_lowercase();
    (Some(actual), matched)
}

fn sha256_of(path: &Path) -> Option<String> {
    use std::io::Read as _;

    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::with_capacity(1 << 16, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 16];
    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .fold(String::with_capacity(64), |mut out, byte| {
                use std::fmt::Write as _;
                let _ = write!(out, "{byte:02x}");
                out
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-spike-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn layout_follows_the_production_naming_convention() {
        let layout = Layout::new(Path::new("/data/app"));
        assert_eq!(layout.data_dir, Path::new("/data/app/corpus"));
        assert_eq!(
            layout.archive,
            Path::new("/data/app/corpus/yunjian-corpus.db.gz"),
            "归档名必须来自 CORPUS_ARCHIVE_NAME，不能在这里另起一个"
        );
        assert_eq!(
            layout.sidecar,
            Path::new("/data/app/corpus/yunjian-corpus.db.gz.sha256"),
            "旁文件是归档名直接追加 .sha256，改成 with_extension 会吃掉 .gz"
        );
        assert_eq!(layout.corpus, Path::new("/data/app/corpus/corpus.db"));
    }

    #[test]
    fn observe_times_both_milestones_in_order() {
        let root = scratch("milestones");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(&layout.data_dir).expect("建落地目录");
        let (tx, rx) = channel();
        let sidecar = layout.sidecar.clone();
        let corpus = layout.corpus.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            std::fs::write(&sidecar, "abc  yunjian-corpus.db.gz\n").expect("写旁文件");
            std::thread::sleep(Duration::from_millis(250));
            std::fs::write(&corpus, b"corpus").expect("写语料");
            let _ = tx.send(Ok(()));
        });

        let seen = observe(
            &layout,
            &rx,
            Duration::from_secs(10),
            Duration::from_millis(20),
        );
        let download = seen.download_verified.expect("必须看到下载校验里程碑");
        let total = seen.materialized.expect("必须看到落盘里程碑");
        assert!(
            download >= Duration::from_millis(200),
            "下载里程碑不该早于它真正发生的时刻：{download:?}"
        );
        assert!(
            total > download,
            "落盘必须晚于下载校验：total={total:?} download={download:?}"
        );
        assert!(seen.worker_error.is_none());
        assert!(!seen.timed_out);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn observe_reports_a_production_failure_without_inventing_timings() {
        let root = scratch("worker-error");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(&layout.data_dir).expect("建落地目录");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let _ = tx.send(Err("清单里的摘要与实际不符".to_owned()));
        });

        let seen = observe(
            &layout,
            &rx,
            Duration::from_secs(5),
            Duration::from_millis(20),
        );
        assert_eq!(
            seen.worker_error.as_deref(),
            Some("清单里的摘要与实际不符"),
            "生产路径的失败原因必须原样带出来，而不是替它编一个"
        );
        assert!(seen.materialized.is_none(), "没落盘就不能有落盘耗时");
        assert!(seen.download_verified.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn observe_marks_a_budget_overrun_instead_of_hanging() {
        let root = scratch("budget");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(&layout.data_dir).expect("建落地目录");
        // 发送端保持存活，否则 observe 会走 Disconnected 分支而不是超时分支。
        let (_tx, rx) = channel::<Result<(), String>>();
        let seen = observe(
            &layout,
            &rx,
            Duration::from_millis(120),
            Duration::from_millis(20),
        );
        assert!(seen.timed_out, "预算耗尽必须显式标记，不能静默返回空测量");
        assert!(seen.materialized.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn observe_still_credits_a_materialization_that_lands_just_before_the_worker_returns() {
        let root = scratch("late-landing");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(&layout.data_dir).expect("建落地目录");
        std::fs::write(&layout.sidecar, "abc  yunjian-corpus.db.gz\n").expect("写旁文件");
        std::fs::write(&layout.corpus, b"corpus").expect("写语料");
        let (tx, rx) = channel();
        tx.send(Ok(())).expect("投递结果");
        let seen = observe(
            &layout,
            &rx,
            Duration::from_secs(5),
            Duration::from_millis(20),
        );
        assert!(
            seen.materialized.is_some(),
            "线程先返回、文件已就绪时不能报成未落盘"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn residual_temp_files_only_counts_the_production_temp_suffix() {
        let root = scratch("residual");
        let dir = root.join("corpus");
        std::fs::create_dir_all(&dir).expect("建目录");
        std::fs::write(dir.join("corpus.db"), b"x").expect("写语料");
        std::fs::write(dir.join("corpus.db.deriving"), b"x").expect("写派生标记");
        assert_eq!(
            residual_temp_files(&dir),
            0,
            "派生标记不是临时文件，把它算进残留会让原子性判定误红"
        );
        std::fs::write(dir.join("yunjian-corpus.db.gz.42.0.tmp"), b"x").expect("写临时文件");
        assert_eq!(residual_temp_files(&dir), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recorded_digest_is_recomputed_rather_than_transcribed() {
        let root = scratch("digest");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(&layout.data_dir).expect("建落地目录");
        std::fs::write(&layout.archive, b"yunjian").expect("写归档");
        let actual = sha256_of(&layout.archive).expect("算摘要");
        std::fs::write(&layout.sidecar, format!("{actual}  yunjian-corpus.db.gz\n"))
            .expect("写旁文件");
        let (reported, verified) = verify_recorded_digest(&layout);
        assert_eq!(reported.as_deref(), Some(actual.as_str()));
        assert!(verified);

        std::fs::write(&layout.sidecar, "00  yunjian-corpus.db.gz\n").expect("改旁文件");
        let (reported, verified) = verify_recorded_digest(&layout);
        assert_eq!(
            reported.as_deref(),
            Some(actual.as_str()),
            "报告里的摘要必须是实测值，不能跟着旁文件一起变"
        );
        assert!(!verified, "旁文件与实际不符时不能报 sha256_verified=true");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreachable_manifest_is_a_reported_failure_not_a_panic() {
        let root = scratch("unreachable");
        let missing = root.join("nope").join("assets_manifest.json");
        let measured = measure(
            &missing.to_string_lossy(),
            &root,
            Duration::from_millis(400),
        );
        assert!(measured.crashed, "拿不到清单必须如实记成失败");
        assert!(
            measured.failure.is_some(),
            "失败必须带原因，否则读者无从下手"
        );
        assert!(measured.duration_seconds.is_none(), "失败时不能编出耗时");
        assert!(!measured.sha256_verified);
        assert!(!measured.derive_awaited, "永远不等派生，且必须写出来");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn measurement_json_carries_every_key_the_gate_requires() {
        let measured = Measurement::failed("", "桩".to_owned());
        let value: serde_json::Value =
            serde_json::from_str(&measured.to_json()).expect("测量必须是合法 JSON");
        for key in [
            "artifact_bytes",
            "sha256_verified",
            "duration_seconds",
            "atomic_install",
            "crashed",
            "production_path",
        ] {
            assert!(
                value.get(key).is_some(),
                "判据②声明的必需字段 `{key}` 不在 JSON 里，宿主侧会判成未执行"
            );
        }
        assert_eq!(
            value["production_path"], PRODUCTION_PATH,
            "报告必须点名被调用的生产符号"
        );
    }
}
