//! 语料获取：把首启落地与派生包进工作区统一的长任务协议。
//!
//! # 为什么每条命令都走这里
//!
//! 首启路径要做校验、解压和派生三件事，实测唐宋规模合计约十分钟（其中派生 571.8 s）。
//! 一条静默阻塞十分钟的命令与卡死无法区分，所以每条需要语料的命令都从这里取语料，
//! 而不是只有 `corpus fetch` 才汇报进度。
//!
//! # 为什么用 [`yunjian_core::operation`] 而不是直接回调
//!
//! [`yunjian_core::CorpusHandle::open_with_progress`] 的回调在工作线程上同步执行，
//! 回调里做的事会直接拖慢派生。协议把生产与消费分开：工作线程只往有界队列里塞事件，
//! 主线程按自己的节奏取。**进度事件会合并成最新一条**（协议规定），所以百分比这种
//! 高频事件不会堆积；而「归档已校验」「已落地」这类里程碑必须一条不丢，因此走
//! `Item` 而不是 `Progress`。
//!
//! 事件都是自持数据：[`yunjian_core::MaterializationProgress`] 借用调用栈上的路径与
//! 字符串，跨线程送不出去。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use yunjian_core::{
    CorpusConfig, CorpusHandle, DeriveProgress, DeriveStep, DerivedState, MaterializationProgress,
    operation::{Event, next_event, start_operation},
};

/// 轮询一次事件的等待上限。
///
/// 500 ms 不影响吞吐（事件到达即唤醒），只决定「一条事件都没有」时多久回一次主循环。
const POLL_TIMEOUT_MS: u64 = 500;

/// 可合并的高频进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// 正在解压归档。`bytes_total == 0` 表示清单没给出解压后大小。
    Decompressing {
        /// 已写出的字节数。
        bytes_done: u64,
        /// 预期总字节数；未知时为零。
        bytes_total: u64,
    },
    /// 正在派生检索结构。
    Deriving {
        /// 当前阶段。
        step: DeriveStep,
        /// 当前阶段已处理的作品数。
        done: u64,
        /// 当前阶段的作品总数；未知时为零。
        total: u64,
    },
}

/// 不可丢弃的里程碑。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Milestone {
    /// 已有可用语料库，本次没有落地动作。
    AlreadyPresent {
        /// 已存在的语料库路径。
        path: String,
    },
    /// 开始校验归档。
    VerifyingArchive {
        /// 归档路径。
        archive: String,
        /// 归档实际字节数。
        bytes: u64,
    },
    /// 归档摘要与期望一致。
    ArchiveVerified {
        /// 已确认的 SHA-256。
        sha256: String,
    },
    /// 已原子落地。
    Materialized {
        /// 落地后的语料库路径。
        path: String,
        /// 语料数据版本。
        corpus_version: String,
    },
    /// 首启派生失败。字典仍可用，两字查询退化，下次运行会重来。
    DeriveFailed {
        /// 失败原因。
        reason: String,
    },
    /// 语料库已就绪。
    Ready {
        /// 可查询的语料库路径。
        path: String,
        /// 语料数据版本。
        corpus_version: String,
        /// 派生结构是否就绪。
        derived: bool,
    },
}

/// 一次成功的语料获取。
#[derive(Debug)]
pub struct Provisioned {
    /// 已就绪的只读语料库。
    pub handle: CorpusHandle,
    /// 本次经过的里程碑，按发生顺序。
    pub milestones: Vec<Milestone>,
}

impl Provisioned {
    /// 本次运行是否真的落地了一份新语料。
    #[must_use]
    pub fn materialized(&self) -> bool {
        self.milestones
            .iter()
            .any(|milestone| matches!(milestone, Milestone::Materialized { .. }))
    }

    /// 首启派生失败的原因；成功或无需派生时为 `None`。
    #[must_use]
    pub fn derive_failure(&self) -> Option<&str> {
        self.milestones
            .iter()
            .find_map(|milestone| match milestone {
                Milestone::DeriveFailed { reason } => Some(reason.as_str()),
                _ => None,
            })
    }
}

/// 按解析顺序取语料，必要时校验、落地并派生。
///
/// 进度与里程碑一律以 `tracing` 记到 stderr：stdout 属于结果与 MCP 协议流，进度条写
/// 到那里会毁掉 `--json | jq` 与 MCP 会话。
///
/// # Errors
///
/// 语料无法定位、校验失败、解压失败或打开失败时返回中文原因。调用方一律映射到退出码 3
/// ——能走到这里的失败没有一种是「命令写错了」。
pub fn provision(config: &CorpusConfig) -> Result<Provisioned, String> {
    let owned = config.clone();
    let slot: Arc<Mutex<Option<CorpusHandle>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);

    let operation = start_operation::<Progress, Milestone, _>(move |reporter| {
        let handle = CorpusHandle::open_with_progress(&owned, &mut |event| {
            // 送不出去（消费端已关闭）时无需处理：句柄一旦关闭，整个操作也就没有消费方了，
            // 协议会在生产者返回时给出终态。
            if let Some(milestone) = Milestone::from_progress(event) {
                let _ = reporter.item(milestone);
            } else if let Some(progress) = Progress::from_progress(event) {
                let _ = reporter.progress(progress);
            }
        })
        .map_err(|error| error.to_string())?;
        store(&sink, handle);
        Ok(())
    });

    let mut milestones = Vec::new();
    loop {
        match next_event(&operation, POLL_TIMEOUT_MS) {
            Some(Event::Progress(progress)) => log_progress(progress),
            Some(Event::Item(milestone)) => {
                log_milestone(&milestone);
                milestones.push(milestone);
            }
            Some(Event::Done) => break,
            Some(Event::Failed { message }) => return Err(message),
            // 本 CLI 不发起取消；协议要求 `Cancelled` 可达，故如实处理而不是断言不可能。
            Some(Event::Cancelled) => return Err("语料获取被取消".to_owned()),
            None => {}
        }
    }

    take(&slot).map_or_else(
        || Err("语料获取已完成但没有交回句柄".to_owned()),
        |handle| Ok(Provisioned { handle, milestones }),
    )
}

/// 把派生状态翻成一句面向用户的降级说明；就绪时为 `None`。
#[must_use]
pub fn degradation(state: &DerivedState) -> Option<String> {
    match state {
        DerivedState::Ready { .. } => None,
        DerivedState::Unavailable { reason } => Some(format!(
            "首启派生结构不可用（{reason}），一至两字查询本次退化为全表扫描；下次启动会自动重试"
        )),
    }
}

impl Milestone {
    fn from_progress(event: MaterializationProgress<'_>) -> Option<Self> {
        match event {
            MaterializationProgress::AlreadyPresent { path } => Some(Self::AlreadyPresent {
                path: path.display().to_string(),
            }),
            MaterializationProgress::VerifyingArchive { archive, bytes } => {
                Some(Self::VerifyingArchive {
                    archive: archive.display().to_string(),
                    bytes,
                })
            }
            MaterializationProgress::ArchiveVerified { sha256 } => Some(Self::ArchiveVerified {
                sha256: sha256.to_owned(),
            }),
            MaterializationProgress::Materialized {
                path,
                corpus_version,
            } => Some(Self::Materialized {
                path: path.display().to_string(),
                corpus_version: corpus_version.to_owned(),
            }),
            MaterializationProgress::DeriveFailed { reason } => Some(Self::DeriveFailed {
                reason: reason.to_owned(),
            }),
            MaterializationProgress::Ready {
                path,
                corpus_version,
                derived,
            } => Some(Self::Ready {
                path: path.display().to_string(),
                corpus_version: corpus_version.to_owned(),
                derived,
            }),
            MaterializationProgress::Decompressing { .. }
            | MaterializationProgress::Deriving(_) => None,
        }
    }
}

impl Progress {
    fn from_progress(event: MaterializationProgress<'_>) -> Option<Self> {
        match event {
            MaterializationProgress::Decompressing {
                bytes_done,
                bytes_total,
            } => Some(Self::Decompressing {
                bytes_done,
                bytes_total,
            }),
            MaterializationProgress::Deriving(DeriveProgress { step, done, total }) => {
                Some(Self::Deriving { step, done, total })
            }
            // 两侧都穷举：上游新增事件时这里与 `Milestone::from_progress` 会同时编译失败，
            // 于是「它是里程碑还是进度」必须被回答一次，而不能因为落进通配分支而静默丢失。
            MaterializationProgress::AlreadyPresent { .. }
            | MaterializationProgress::VerifyingArchive { .. }
            | MaterializationProgress::ArchiveVerified { .. }
            | MaterializationProgress::Materialized { .. }
            | MaterializationProgress::DeriveFailed { .. }
            | MaterializationProgress::Ready { .. } => None,
        }
    }
}

fn store(slot: &Mutex<Option<CorpusHandle>>, handle: CorpusHandle) {
    let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(handle);
}

fn take(slot: &Mutex<Option<CorpusHandle>>) -> Option<CorpusHandle> {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// 进度日志的计数器。
///
/// 事件已由协议合并成最新一条，但解压与派生仍可能每秒来几条；`info` 级别每条都打会把
/// 一次首启变成上千行日志。因此进度走 `debug`，只有每 32 条抽一条上 `info`——这样默认
/// 级别下用户仍能看出「它在动」，而 `--log-level debug` 能看到全部。
static PROGRESS_TICKS: AtomicU64 = AtomicU64::new(0);

/// 每多少条进度事件抽一条到 `info`。
const INFO_EVERY: u64 = 32;

fn log_progress(progress: Progress) {
    let tick = PROGRESS_TICKS.fetch_add(1, Ordering::Relaxed);
    let loud = tick.is_multiple_of(INFO_EVERY);
    match progress {
        Progress::Decompressing {
            bytes_done,
            bytes_total,
        } => {
            let percent = percent(bytes_done, bytes_total);
            if loud {
                tracing::info!(bytes_done, bytes_total, percent, "正在解压语料归档");
            } else {
                tracing::debug!(bytes_done, bytes_total, percent, "正在解压语料归档");
            }
        }
        Progress::Deriving { step, done, total } => {
            let percent = percent(done, total);
            if loud {
                tracing::info!(
                    step = step.display_name(),
                    done,
                    total,
                    percent,
                    "正在派生检索结构"
                );
            } else {
                tracing::debug!(
                    step = step.display_name(),
                    done,
                    total,
                    percent,
                    "正在派生检索结构"
                );
            }
        }
    }
}

fn log_milestone(milestone: &Milestone) {
    match milestone {
        Milestone::AlreadyPresent { path } => {
            tracing::debug!(path = %path, "语料库已存在，无需落地");
        }
        Milestone::VerifyingArchive { archive, bytes } => {
            tracing::info!(archive = %archive, bytes, "正在校验语料归档摘要");
        }
        Milestone::ArchiveVerified { sha256 } => {
            tracing::info!(sha256 = %sha256, "语料归档摘要一致");
        }
        Milestone::Materialized {
            path,
            corpus_version,
        } => {
            tracing::info!(path = %path, corpus_version = %corpus_version, "语料库已落地");
        }
        Milestone::DeriveFailed { reason } => {
            tracing::warn!(reason = %reason, "首启派生失败，两字查询本次退化");
        }
        Milestone::Ready {
            path,
            corpus_version,
            derived,
        } => {
            tracing::debug!(path = %path, corpus_version = %corpus_version, derived, "语料库就绪");
        }
    }
}

/// 百分比；总量未知时返回 -1 而不是 0——0% 会被读成「一点没动」。
fn percent(done: u64, total: u64) -> i64 {
    if total == 0 {
        return -1;
    }
    let ratio = (done as f64 / total as f64).clamp(0.0, 1.0) * 100.0;
    ratio.round() as i64
}

#[cfg(test)]
mod tests {
    use super::{Milestone, Progress, degradation, percent, provision};
    use std::path::Path;
    use yunjian_core::{CorpusConfig, DeriveStep, DerivedState, MaterializationProgress};

    #[test]
    fn milestones_and_progress_partition_every_materialization_event() {
        let path = Path::new("/tmp/corpus.db");
        let events = [
            MaterializationProgress::AlreadyPresent { path },
            MaterializationProgress::VerifyingArchive {
                archive: path,
                bytes: 1,
            },
            MaterializationProgress::ArchiveVerified { sha256: "abc" },
            MaterializationProgress::Materialized {
                path,
                corpus_version: "v1",
            },
            MaterializationProgress::DeriveFailed {
                reason: "磁盘满"
            },
            MaterializationProgress::Ready {
                path,
                corpus_version: "v1",
                derived: true,
            },
            MaterializationProgress::Decompressing {
                bytes_done: 1,
                bytes_total: 2,
            },
            MaterializationProgress::Deriving(yunjian_core::DeriveProgress {
                step: DeriveStep::Ngram,
                done: 1,
                total: 2,
            }),
        ];
        for event in events {
            let milestone = Milestone::from_progress(event);
            let progress = Progress::from_progress(event);
            // 每个事件恰好落进一侧：里程碑不许被合并掉，进度不许挤占里程碑队列。
            assert_ne!(
                milestone.is_some(),
                progress.is_some(),
                "事件 {event:?} 必须恰好归入里程碑或进度之一"
            );
        }
    }

    #[test]
    fn a_missing_corpus_reports_a_reason_instead_of_panicking() {
        let error = provision(&CorpusConfig {
            path: Some("/nonexistent/yunjian-corpus.db".into()),
            data_dir: "/nonexistent".into(),
            archive: None,
        })
        .expect_err("不存在的语料库必须报错");
        assert!(error.contains("/nonexistent"), "{error}");
    }

    #[test]
    fn degradation_explains_itself_and_stays_silent_when_ready() {
        assert!(degradation(&DerivedState::Ready { stats: None }).is_none());
        let text = degradation(&DerivedState::Unavailable {
            reason: "磁盘只读".to_owned(),
        })
        .expect("退化必须有说明");
        assert!(text.contains("磁盘只读"), "必须透出原始原因：{text}");
        assert!(text.contains("退化"), "必须说清后果：{text}");
    }

    #[test]
    fn unknown_total_reports_minus_one_rather_than_zero_percent() {
        assert_eq!(percent(0, 0), -1);
        assert_eq!(percent(5, 0), -1);
        assert_eq!(percent(1, 2), 50);
        assert_eq!(percent(9, 3), 100);
    }
}
