//! DOM 层驱动步骤。每条断言在这里被**真的走一遍**，而不是记一条「尚未实现」。
//!
//! # 一次会话驱动不完这六条
//!
//! 三条断言各自要求一个互相冲突的**前置环境**：
//!
//! | 断言 | 前置 |
//! | --- | --- |
//! | `corpus_first_run_materialization` | 一个**空的**应用数据目录（否则语料已物化，首启不会发生） |
//! | `voice_degradation_states_reason` | 一个**取不到语音模型**的环境 |
//! | 其余四条 | 真实数据目录：已物化语料、随包赏析表、真实模型目录 |
//!
//! 环境是进程启动时定下来的，一个活着的 WebView 里改不了。所以这里开三次会话，每次
//! 用不同的 env 拉起同一个产物。**不是为了方便**：把首启物化放进一个语料已经在的会话里
//! 只能观测到「按钮点了没报错」，而那不是这条断言要证明的事。
//!
//! # 环境是怎么被换掉的
//!
//! 写一份临时 `config.toml` 并用 `APP_CONFIG` 指向它，把 `app.data_dir` 与
//! `corpus.data_dir` 指到一个空目录。走的仍是**生产那条解析路径**
//! （应用数据目录 → 校验 `.db.gz` 的 SHA-256 → 原子解压）。
//!
//! 两条路刻意没走：
//!
//! - **不换 `HOME`**。换掉它会顺带换掉 `tauri-driver` 自己看到的 `HOME`，而本机 PATH 上
//!   那个 `tauri-driver` 是一条 mise shim——它读不到 `~/.config/mise/config.toml` 就直接
//!   拒绝执行，一个字节的应用代码都不会跑到。症状是「driver 起来后没有监听端口」，
//!   读起来像 WebDriver 环境不行，真因是 harness 自己把 shim 的 `HOME` 抽掉了。
//! - **不用 `YUNJIAN_CORPUS_PATH`**。那条直接指定一个已有文件、跳过物化，
//!   于是测不到要测的东西。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::webdriver::{self, Session};
use super::{Collector, Verdict};
use crate::verify_sources::emit;

/// 等 React 把首屏挂到 DOM 上的上限。实测约 2 秒，取余量。
const MOUNT_TIMEOUT: Duration = Duration::from_secs(20);
/// 等一次检索出摘要的上限。本机实测 5.09 秒（debug 构建、47 万首、FTS5）。
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
/// 等首启物化走完的上限。223 MiB 的 `.db.gz` 要校验 SHA-256 再解压成 4.7 GiB。
const MATERIALIZE_TIMEOUT: Duration = Duration::from_secs(900);
/// 等语音可用性探测出结论的上限。
const VOICE_PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// 等一轮语音跟读走完的上限。示范合成加逐句识别。
const VOICE_ROUND_TIMEOUT: Duration = Duration::from_secs(300);
/// 等上一条会话的应用退干净的上限。
const QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);

const SEARCH_INPUT: &str = "[data-testid='search-input']";
const SEARCH_SUBMIT: &str = "[data-testid='search-submit']";
const SEARCH_SUMMARY: &str = "[data-testid='search-summary']";
const RESULT_ROW: &str = "[data-testid='result-row']";
const ROW_TITLE: &str = ".result-row__title";
const ROW_OPEN: &str = ".result-row__open";

struct Outcome {
    verdict: Verdict,
    detail: String,
    executable_when: Option<String>,
}

impl Outcome {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Pass,
            detail: detail.into(),
            executable_when: None,
        }
    }

    fn fail(detail: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Fail,
            detail: detail.into(),
            executable_when: None,
        }
    }

    fn not_executed(detail: impl Into<String>, when: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::NotExecuted,
            detail: detail.into(),
            executable_when: Some(when.into()),
        }
    }
}

pub(crate) struct SessionEnv {
    display: String,
    config: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    extra: Vec<(String, String)>,
}

impl SessionEnv {
    pub(crate) fn new(display: &str) -> Self {
        Self {
            display: display.to_owned(),
            config: None,
            model_dir: None,
            extra: Vec::new(),
        }
    }

    /// 追加产物自身布局要求的环境变量（见 `linux::library_env`）。
    pub(crate) fn with_extra(mut self, extra: Vec<(String, String)>) -> Self {
        self.extra = extra;
        self
    }

    pub(crate) fn with_config(mut self, config: PathBuf) -> Self {
        self.config = Some(config);
        self
    }

    pub(crate) fn with_model_dir(mut self, dir: PathBuf) -> Self {
        self.model_dir = Some(dir);
        self
    }

    fn to_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = vec![
            ("DISPLAY", self.display.clone()),
            ("TAURI_WEBVIEW_AUTOMATION", "true".to_owned()),
            ("YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK", "1".to_owned()),
        ];
        if let Some(config) = &self.config {
            pairs.push(("APP_CONFIG", config.to_string_lossy().into_owned()));
        }
        if let Some(dir) = &self.model_dir {
            pairs.push(("YUNJIAN_MODEL_DIR", dir.to_string_lossy().into_owned()));
        }
        pairs
    }

    fn extra_pairs(&self) -> &[(String, String)] {
        &self.extra
    }
}

/// 主数据目录那一路：两字检索、输入法预填、随包赏析、语音一轮。
pub(crate) fn drive_primary(
    driver: &Session,
    shots: &Path,
    capture: &super::linux::CaptureProbe,
    collector: &mut Collector,
) -> Result<()> {
    let mounted = driver.wait_for(SEARCH_INPUT, MOUNT_TIMEOUT);

    let two_char = if mounted {
        two_char_search(driver)
    } else {
        Outcome::fail(format!(
            "等了 {} 秒，`{SEARCH_INPUT}` 仍未出现在 DOM 里",
            MOUNT_TIMEOUT.as_secs()
        ))
    };
    record(
        driver,
        collector,
        "two_char_search_returns_results",
        two_char,
        shots,
        "two-char-search.png",
    )?;

    let ime = if mounted {
        ime_into_prefilled_box(driver)
    } else {
        Outcome::not_executed(
            "首屏未挂载，检索框不存在，往它输入无从执行",
            "一次 React 首屏挂载成功的会话",
        )
    };
    record(
        driver,
        collector,
        "ime_prefilled_search_box",
        ime,
        shots,
        "ime-prefilled-dom.png",
    )?;

    let appreciation = if mounted {
        shipped_appreciation(driver)
    } else {
        Outcome::not_executed(
            "首屏未挂载，无法从检索进入详情页",
            "一次 React 首屏挂载成功的会话",
        )
    };
    record_focused(
        driver,
        collector,
        "shipped_appreciation_without_key",
        appreciation,
        shots,
        "shipped-appreciation.png",
        Some("[data-testid='ai-panel']"),
    )?;

    let voice = if mounted {
        voice_round(driver, capture)
    } else {
        Outcome::not_executed(
            "首屏未挂载，无法进入背诵页",
            "一次 React 首屏挂载成功的会话",
        )
    };
    record(
        driver,
        collector,
        "voice_round_succeeds_end_to_end",
        voice,
        shots,
        "voice-round.png",
    )?;
    Ok(())
}

/// 空数据目录那一路：首启语料物化。
pub(crate) fn drive_first_run(
    driver: &Session,
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let outcome = corpus_first_run(driver);
    // 语料那条的判词在设置弹窗里的折叠线以下。聚焦到语料区那三种终态各自的元素：
    // 失败时是 `corpus-error`，成功时是 `corpus-facts`，都还没出现时退回 `fetch-corpus`
    // 那个按钮。**没有 `corpus-panel` 这个 testid**——凭记忆写标识符正是本项目栽过的那一类，
    // 这三个名字逐一 grep 自 `app/src/settings/CorpusPanel.tsx`。
    let focus = ["corpus-error", "corpus-facts", "fetch-corpus"]
        .into_iter()
        .map(|id| format!("[data-testid='{id}']"))
        .find(|css| driver.find(css).is_ok());
    record_focused(
        driver,
        collector,
        "corpus_first_run_materialization",
        outcome,
        shots,
        "corpus-first-run.png",
        focus.as_deref(),
    )
}

/// 取不到模型那一路：语音降级并说明原因。
pub(crate) fn drive_degradation(
    driver: &Session,
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let outcome = voice_degradation(driver);
    record(
        driver,
        collector,
        "voice_degradation_states_reason",
        outcome,
        shots,
        "voice-degradation.png",
    )
}

/// 为首启物化准备一个空数据目录，并写一份指向它的 `config.toml`，返回配置文件路径。
///
/// 目录里只预先放生产路径要读的那个 `.db.gz` 与它的 `.sha256`——物化正是从这里开始的。
/// 只做**硬链接**，不复制：那个文件 223 MiB，而首启本来就要把它解压成 4.7 GiB，
/// 再多一份拷贝没有换来任何被验证的事实。跨文件系统时才退回复制。
///
/// # Errors
///
/// 本机那份 `.db.gz` 或它的校验和不存在（说明从未取过语料），或建目录、写配置失败。
pub(crate) fn seed_first_run(real_data_root: &Path, fresh_root: &Path) -> Result<PathBuf> {
    let source_dir = real_data_root.join("corpus");
    let app_dir = fresh_root.join("app");
    let corpus_dir = fresh_root.join("corpus");
    let log_dir = fresh_root.join("logs");
    for dir in [&app_dir, &corpus_dir, &log_dir] {
        std::fs::create_dir_all(dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    }

    for name in ["yunjian-corpus.db.gz", "yunjian-corpus.db.gz.sha256"] {
        let source = source_dir.join(name);
        if !source.is_file() {
            bail!(
                "{} 不存在：首启物化要读的是生产路径上那个待校验的 `.db.gz`，\
                 本机没有它就无法真的走一遍首启",
                source.display()
            );
        }
        let target = corpus_dir.join(name);
        if target.exists() {
            std::fs::remove_file(&target)
                .with_context(|| format!("清理上一次的 {} 失败", target.display()))?;
        }
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target)
                .with_context(|| format!("复制 {} 失败", source.display()))?;
        }
    }

    let config = fresh_root.join("config.toml");
    let text = format!(
        "[app]\nname = \"yunjian\"\ndata_dir = {app}\n\n\
         [corpus]\ndata_dir = {corpus}\n\n\
         [logger]\ndir = {logs}\n",
        app = toml_path(&app_dir),
        corpus = toml_path(&corpus_dir),
        logs = toml_path(&log_dir),
    );
    std::fs::write(&config, text).with_context(|| format!("写 {} 失败", config.display()))?;
    Ok(config)
}

/// TOML 里的路径字面量。用双引号并转义反斜杠，Windows 路径也不会被读成转义序列。
fn toml_path(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy().replace('\\', "\\\\"))
}

fn two_char_search(driver: &Session) -> Outcome {
    let summary = (|| -> Result<String> {
        driver.send_keys(SEARCH_INPUT, "明月")?;
        driver.click(SEARCH_SUBMIT)?;
        if !driver.wait_for(SEARCH_SUMMARY, SEARCH_TIMEOUT) {
            bail!(
                "等了 {} 秒，`{SEARCH_SUMMARY}` 仍未出现在 DOM 里",
                SEARCH_TIMEOUT.as_secs()
            );
        }
        driver.text(SEARCH_SUMMARY)
    })();
    match summary {
        Ok(text) if !text.trim().is_empty() => {
            Outcome::pass(format!("检索「明月」后摘要为「{text}」"))
        }
        Ok(_) => Outcome::fail("检索「明月」后 search-summary 为空"),
        Err(error) => Outcome::fail(format!("驱动检索失败：{error}")),
    }
}

/// 往一个**已有内容**的检索框里再输入中文，读回框里实际是什么（tauri#15436）。
///
/// 这条与 OS 通道那条 `ime_prefilled_search_box_no_freeze` 是**互补**而不是重复：
/// 那条只能证明界面没冻住（窗口还响应状态迁移），证不了字符落在哪；本条读
/// `input.value`，因此能逮到「界面还活着但字被吞了」——那正是该缺陷最坏的形态。
///
/// 判据是**逐字符相等**，不是「非空」：吞掉一半字符同样是缺陷，而非空判据会放过它。
fn ime_into_prefilled_box(driver: &Session) -> Outcome {
    const FIRST: &str = "明月";
    const SECOND: &str = "千里";

    let observed = (|| -> Result<(String, String)> {
        // 先清空，别让上一条断言留下的内容混进来：本条要的是「框里已有内容」这个前置态，
        // 而它必须由本条自己摆好，否则判词说的是另一个起始状态。
        driver.clear(SEARCH_INPUT)?;
        driver.send_keys(SEARCH_INPUT, FIRST)?;
        let prefilled = driver.property(SEARCH_INPUT, "value")?;
        driver.click(SEARCH_INPUT)?;
        driver.send_keys(SEARCH_INPUT, SECOND)?;
        Ok((prefilled, driver.property(SEARCH_INPUT, "value")?))
    })();

    match observed {
        Ok((prefilled, _)) if prefilled != FIRST => Outcome::fail(format!(
            "预填这一步就没成立：往空检索框输入「{FIRST}」之后 `input.value` 是「{prefilled}」；\
             「已有内容的框」这个前置态未建立，后一半无从判定"
        )),
        Ok((_, after)) if after == format!("{FIRST}{SECOND}") => Outcome::pass(format!(
            "检索框预填「{FIRST}」后再次聚焦并输入「{SECOND}」，\
             `input.value` 为「{after}」——字符全部落入框内，无吞字"
        )),
        Ok((_, after)) => Outcome::fail(format!(
            "检索框预填「{FIRST}」后再次聚焦并输入「{SECOND}」，\
             `input.value` 却是「{after}」，期望「{FIRST}{SECOND}」；\
             字符没有全部落入已有内容的检索框（tauri#15436 的形态）"
        )),
        Err(error) => Outcome::fail(format!("驱动输入法预填输入失败：{error}")),
    }
}

/// 首启语料物化：点「下载语料库」，等它走完，读回它显示了什么。
///
/// 这条会话的 `HOME` 是空的，所以状态必然从 `corpus-absent` 开始——**先断言这一点**，
/// 否则在一个语料已经在的目录里点「检查更新」也会得到 ready，而那不是首启。
fn corpus_first_run(driver: &Session) -> Outcome {
    let observed = (|| -> Result<FirstRun> {
        if !driver.wait_for("[data-testid='nav-settings']", MOUNT_TIMEOUT) {
            bail!("等了 {} 秒，主导航仍未挂载", MOUNT_TIMEOUT.as_secs());
        }
        driver.click("[data-testid='nav-settings']")?;
        if !driver.wait_for("[data-testid='settings-dialog']", MOUNT_TIMEOUT) {
            bail!("点「设置」后设置弹窗没有出现");
        }
        let absent = driver.wait_for("[data-testid='corpus-absent']", MOUNT_TIMEOUT);
        driver.click("[data-testid='fetch-corpus']")?;

        let deadline = Instant::now() + MATERIALIZE_TIMEOUT;
        let mut progress_seen = Vec::new();
        loop {
            if let Ok(text) = driver.text("[data-testid='corpus-facts']") {
                return Ok(FirstRun::Ready {
                    absent_first: absent,
                    facts: text,
                    progress_seen,
                });
            }
            if let Ok(text) = driver.text("[data-testid='corpus-error']") {
                return Ok(FirstRun::Refused {
                    absent_first: absent,
                    message: text,
                });
            }
            // 任何一个进度元素都算：本条要判的是「物化过程中界面说了些什么」，
            // 而不是某一个特定 testid 必须存在。
            for css in [
                "[data-testid='corpus-progress']",
                "[data-testid='corpus-materializing']",
                "[data-testid='corpus-busy']",
            ] {
                if let Ok(text) = driver.text(css)
                    && !text.trim().is_empty()
                    && !progress_seen.iter().any(|seen: &String| seen == &text)
                {
                    progress_seen.push(text);
                }
            }
            if Instant::now() >= deadline {
                return Ok(FirstRun::TimedOut {
                    absent_first: absent,
                });
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    })();

    match observed {
        Ok(FirstRun::Ready {
            absent_first,
            facts,
            progress_seen,
        }) => {
            let start = if absent_first {
                "起始状态是 `corpus-absent`（空数据目录，确为首启）"
            } else {
                "起始状态不是 `corpus-absent`"
            };
            if progress_seen.is_empty() {
                Outcome::fail(format!(
                    "{start}，物化完成并渲染出 `corpus-facts`（{}），\
                     但**整个过程没有任何进度显示**：`fetch_corpus` 在 Rust 侧确实通过 \
                     `Channel<Event<CorpusProgress, Value>>` 逐步上报进度\
                     （`crates/yunjian-app/src/ipc.rs` 的 `fetch_corpus`），\
                     而前端 `CorpusPort.fetchCorpus()` 调用 `invoke` 时不传任何 channel\
                     （`app/src/data/sampleSettingsPorts.ts`），`CorpusPanel` 也没有进度元素——\
                     后端发出的进度被前端整个丢掉了。本条要求「完成**并显示进度**」，故判失败",
                    one_line(&facts)
                ))
            } else {
                Outcome::pass(format!(
                    "{start}，物化过程显示了进度（{}），完成后渲染出 `corpus-facts`（{}）",
                    progress_seen
                        .iter()
                        .map(|text| format!("「{}」", one_line(text)))
                        .collect::<Vec<_>>()
                        .join("、"),
                    one_line(&facts)
                ))
            }
        }
        Ok(FirstRun::Refused {
            absent_first,
            message,
        }) => Outcome::fail(format!(
            "起始状态 absent={absent_first}；点「下载语料库」之后界面渲染的是错误\
             `corpus-error`：「{}」——首启物化没有完成",
            one_line(&message)
        )),
        Ok(FirstRun::TimedOut { absent_first }) => Outcome::fail(format!(
            "起始状态 absent={absent_first}；点「下载语料库」之后等了 {} 秒，\
             既没有出现 `corpus-facts` 也没有出现 `corpus-error`，界面停在原地",
            MATERIALIZE_TIMEOUT.as_secs()
        )),
        Err(error) => Outcome::fail(format!("驱动首启物化失败：{error}")),
    }
}

enum FirstRun {
    Ready {
        absent_first: bool,
        facts: String,
        progress_seen: Vec<String>,
    },
    Refused {
        absent_first: bool,
        message: String,
    },
    TimedOut {
        absent_first: bool,
    },
}

/// 没有 API key 时随包赏析仍能渲染，且带「AI 赏析」标签与未审校说明。
///
/// 进入路径与真人一致：检索 → 点结果行 → 详情页自动请求赏析。刻意**不**直接调 IPC——
/// 那会绕开这条断言真正要证明的东西（那三个界面元素在用户走到的那个页面上）。
///
/// 选题不靠猜：随包表的主键是 `(stable_id, template_version)`，而 `stable_id` 就是
/// `poem_id`（`ipc.rs` 里 `"poem_id": review.stable_id`），所以先在结果行里按标题找到
/// 那一首，再断言 `ai-source` 是「随包预生成」——命中随包表是本条的前置，而不是碰巧。
fn shipped_appreciation(driver: &Session) -> Outcome {
    const LINE: &str = "秦时明月汉时关";
    const TITLE: &str = "出塞";

    let observed = (|| -> Result<Appreciation> {
        driver.clear(SEARCH_INPUT)?;
        driver.send_keys(SEARCH_INPUT, LINE)?;
        driver.click(SEARCH_SUBMIT)?;
        if !driver.wait_for(SEARCH_SUMMARY, SEARCH_TIMEOUT) {
            bail!("检索「{LINE}」后摘要未出现");
        }
        if !open_row_titled(driver, TITLE)? {
            return Ok(Appreciation::NoSuchPoem);
        }
        if !driver.wait_for("[data-testid='poem-detail']", MOUNT_TIMEOUT) {
            bail!("点开结果行后详情页未渲染");
        }
        if !driver.wait_for("[data-testid='ai-panel']", MOUNT_TIMEOUT) {
            bail!("详情页里 `ai-panel` 未出现");
        }
        let source = driver.text("[data-testid='ai-source']").unwrap_or_default();
        let body = driver.text("[data-testid='ai-text']").ok();
        let label = driver.text("[data-testid='ai-panel-label']").ok();
        let badge = driver.text("[data-testid='ai-unreviewed-badge']").ok();
        let configuration_required = driver
            .text("[data-testid='ai-configuration-required']")
            .ok();
        let absent = driver.text("[data-testid='ai-absent']").ok();
        let failed = driver.text("[data-testid='ai-failed']").ok();
        Ok(Appreciation::Rendered {
            source,
            body,
            label,
            badge,
            configuration_required,
            absent,
            failed,
        })
    })();

    match observed {
        Ok(Appreciation::NoSuchPoem) => Outcome::not_executed(
            format!(
                "检索「{LINE}」的结果行里没有标题为「{TITLE}」的那一首，\
                 因此没能进到一首**随包表里有赏析**的作品；\
                 换一首无随包赏析的诗只会测到「本篇没有随包赏析」，那是另一条路径"
            ),
            "一个随包赏析表与语料库匹配的数据目录（`appreciation_shipped` 的 \
             `stable_id` 必须能在 `poem` 表里找到）",
        ),
        Ok(Appreciation::Rendered {
            configuration_required: Some(message),
            ..
        }) => Outcome::fail(format!(
            "详情页渲染的是「需要先配置 AI 服务商与密钥」而不是随包赏析：「{}」；\
             没有 key 时随包内容应当照常渲染，不该要求配置",
            one_line(&message)
        )),
        Ok(Appreciation::Rendered {
            absent: Some(message),
            ..
        }) => Outcome::fail(format!(
            "未配置 API key 下打开「{TITLE}」，赏析面板渲染的是「{}」——\
             随包表里查不到这一首。面板的标签与未审校说明都在（渲染链路成立），\
             缺的是那次随包命中：`appreciation_shipped` 的查询键是 \
             `(stable_id, template_version)`，且命中后还要 `grounding_digest` 与\
             当次请求算出来的一致，任一不合都会退回本状态",
            one_line(&message)
        )),
        Ok(Appreciation::Rendered {
            failed: Some(message),
            ..
        }) => Outcome::fail(format!(
            "未配置 API key 下打开「{TITLE}」，赏析面板渲染的是失败态：「{}」。\
             随包赏析在真实桌面端从未渲染过，与有没有 key 无关。\
             同一根因还有一个症状：`corpus_first_run_materialization` 那条把 Tauri 的原话\
             逐字带了出来——「invalid args onEvent for command fetch_corpus」。\
             `appreciate_poem` 与 `fetch_corpus` 都声明了一个必需的 \
             `on_event: Channel<..>` 参数，而 `tauriPorts.ts` 的 `appreciate` 与 \
             `sampleSettingsPorts.ts` 的 `fetchCorpus` 都只传了 `request`、没有建 Channel\
             （语音那两条端口是建了的，所以语音走得通）。\
             本条读不到那句原话，是因为详情页的 catch 写的是 \
             `cause instanceof Error ? cause.message : \"AI 赏析获取失败\"`，\
             而 Tauri 拒绝时给的是字符串而不是 Error，于是真实原因被这句兜底文案吞掉——\
             语料面板那侧用的是 `String(cause)`，所以它显示了原话",
            one_line(&message)
        )),
        Ok(Appreciation::Rendered {
            source,
            body,
            label,
            badge,
            ..
        }) => {
            let body_text = body.unwrap_or_default();
            let label_text = label.unwrap_or_default();
            let badge_text = badge.unwrap_or_default();
            let mut missing = Vec::new();
            if body_text.trim().is_empty() {
                missing.push("赏析正文 `ai-text`");
            }
            if label_text != "AI 赏析" {
                missing.push("「AI 赏析」标签 `ai-panel-label`");
            }
            if !badge_text.contains("未经人工审校") {
                missing.push("未审校说明 `ai-unreviewed-badge`");
            }
            if source != "随包预生成" {
                missing.push("随包来源标注 `ai-source`（应为「随包预生成」）");
            }
            if missing.is_empty() {
                Outcome::pass(format!(
                    "未配置任何 API key 下打开「{TITLE}」，`ai-source` 为「{source}」\
                     （命中随包表，非现场生成），标签为「{label_text}」，\
                     并带未审校说明「{}」；正文首段为「{}」。\
                     **正文当前是随包数据集里的未生成标记**，\
                     即本条证明的是渲染与标注链路成立，不是生成能力成立",
                    one_line(&badge_text),
                    clip(&body_text, 40)
                ))
            } else {
                Outcome::fail(format!(
                    "未配置 API key 下打开「{TITLE}」，赏析面板缺少：{}；\
                     实测 `ai-source`=「{source}」、标签=「{label_text}」、\
                     说明=「{}」、正文=「{}」",
                    missing.join("、"),
                    one_line(&badge_text),
                    clip(&body_text, 40)
                ))
            }
        }
        Err(error) => Outcome::fail(format!("驱动随包赏析失败：{error}")),
    }
}

enum Appreciation {
    NoSuchPoem,
    Rendered {
        source: String,
        body: Option<String>,
        label: Option<String>,
        badge: Option<String>,
        configuration_required: Option<String>,
        absent: Option<String>,
        failed: Option<String>,
    },
}

/// 语音一轮端到端：采集 → ASR → **无偏置**评分这条链跑通。
///
/// 「跑通」的判据刻意**不含分数**。2026-08-11 的裁决（`problems.md`）：文言 ASR 实测
/// CER 77.01%，所以 v1 不做机器自动评分，反馈只有「是否开口 / 停顿 / 相对节奏」，
/// FSRS 等级由用户自选。因此本条要求的是那三项反馈都渲染出来，且旁边那句
/// 「不做机器评分」的口径说明在场——如果哪天报告里冒出一个完整度分数，那才是缺陷。
fn voice_round(driver: &Session, capture: &super::linux::CaptureProbe) -> Outcome {
    let observed = (|| -> Result<VoiceRound> {
        enter_voice_mode(driver)?;
        match voice_phase(driver)? {
            VoicePhase::Degraded { reason, message } => {
                return Ok(VoiceRound::Degraded { reason, message });
            }
            VoicePhase::Ready => {}
        }
        driver.click("[data-testid='voice-record']")?;
        let deadline = Instant::now() + VOICE_ROUND_TIMEOUT;
        loop {
            if driver.find("[data-testid='voice-report']").is_ok() {
                return Ok(VoiceRound::Reported {
                    spoke: driver
                        .text("[data-testid='voice-spoke']")
                        .unwrap_or_default(),
                    pauses: driver
                        .text("[data-testid='voice-long-pauses']")
                        .unwrap_or_default(),
                    rhythm: driver
                        .text("[data-testid='voice-relative-rhythm']")
                        .unwrap_or_default(),
                    note: driver
                        .text("[data-testid='voice-no-score-note']")
                        .unwrap_or_default(),
                });
            }
            if let Ok(text) = driver.text("[data-testid='voice-degraded-reason']") {
                return Ok(VoiceRound::Degraded {
                    reason: text,
                    message: driver
                        .text("[data-testid='voice-degraded']")
                        .unwrap_or_default(),
                });
            }
            if let Ok(text) = driver.text("[data-testid='voice-error']")
                && !text.trim().is_empty()
            {
                return Ok(VoiceRound::Errored { message: text });
            }
            if Instant::now() >= deadline {
                return Ok(VoiceRound::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    })();

    match observed {
        Ok(VoiceRound::Reported {
            spoke,
            pauses,
            rhythm,
            note,
        }) => {
            let mut missing = Vec::new();
            if spoke.trim().is_empty() {
                missing.push("是否开口 `voice-spoke`");
            }
            if pauses.trim().is_empty() {
                missing.push("长停顿 `voice-long-pauses`");
            }
            if rhythm.trim().is_empty() {
                missing.push("相对节奏 `voice-relative-rhythm`");
            }
            if !note.contains("不做机器评分") {
                missing.push("「不做机器评分」口径说明 `voice-no-score-note`");
            }
            if missing.is_empty() {
                Outcome::pass(format!(
                    "一轮语音跟读走完：采集与识别都真的发生（`voice-report` 已渲染），\
                     反馈为 是否开口=「{spoke}」、长停顿=「{pauses}」、相对节奏=「{rhythm}」，\
                     且口径说明在场。**报告里没有机器评分**，与 2026-08-11 的裁决一致"
                ))
            } else {
                Outcome::fail(format!(
                    "一轮语音跟读走完并渲染了 `voice-report`，但缺少：{}",
                    missing.join("、")
                ))
            }
        }
        Ok(VoiceRound::Degraded { reason, message }) => {
            let feature_disabled = reason.contains("本版本未编译语音");
            if feature_disabled {
                Outcome::not_executed(
                    format!(
                        "被测产物未编译 `voice` 特性，语音在可用性探测这一步就降级为\
                         「{reason}」，采集与 ASR 都不存在，这一轮无从判定成功；\
                         降级本身由 `voice_degradation_states_reason` 单独判定。\
                         判词：「{}」",
                        one_line(&message)
                    ),
                    "用 `--features custom-protocol,voice` 构建被测产物\
                     （该特性同时是许可边界，见 `docs/VOICE-BUILD.zh.md`），\
                     并提供一个非 monitor 的可采集输入设备",
                )
            } else if capture.usable && is_input_device_reason(&reason) {
                // 采集探测已经**实测**这个会话能录到非静音音频，而应用却说打不开麦克风
                // 或没有麦克风：这两句话不可能同时为真，所以这是产品缺陷而不是环境阻塞。
                //
                // 这条分支正是把「.monitor 该不该算设备」从一个口径问题变成一个可判定
                // 事实的地方：判据不是设备名长什么样，是同一台机器上刚刚录到了什么。
                Outcome::fail(format!(
                    "语音降级为「{reason}」，但本会话的采集探测是成功的——{}。\
                     同一台机器上 harness 刚刚录到了非静音音频，应用却报打不开或没有输入设备，\
                     两者不可能同时为真，故记产品缺陷而非环境阻塞。判词：「{}」",
                    capture.detail,
                    one_line(&message)
                ))
            } else {
                Outcome::not_executed(
                    format!(
                        "语音降级为「{reason}」，因此这一轮没有采集与识别可判；\
                         采集探测结果：{}。判词：「{}」",
                        capture.detail,
                        one_line(&message)
                    ),
                    "一个语音模型就绪、且真实采集探测能录到非静音音频的会话",
                )
            }
        }
        Ok(VoiceRound::Errored { message }) => Outcome::fail(format!(
            "点「开始跟读」之后界面渲染的是 `voice-error`：「{}」",
            one_line(&message)
        )),
        Ok(VoiceRound::TimedOut) => Outcome::fail(format!(
            "点「开始跟读」之后等了 {} 秒，既没有 `voice-report`、也没有降级或错误",
            VOICE_ROUND_TIMEOUT.as_secs()
        )),
        Err(error) => Outcome::fail(format!("驱动语音一轮失败：{error}")),
    }
}

/// 降级原因是否在说「输入设备本身不可用」。
///
/// 只有这几个原因与采集探测的结论**直接矛盾**，因此只有它们会在探测成功时升为 FAIL。
/// 其余原因（模型未就绪、未编译语音、这一次录音未被识别接受……）与「能不能录到声音」
/// 是两件事，探测成功也不能据以判定它们错了。名单逐字取自
/// `app/src/contracts/voice.ts` 的 `DEGRADE_REASON_LABEL`。
fn is_input_device_reason(reason: &str) -> bool {
    const DEVICE_REASONS: [&str; 5] = [
        "没有可用的麦克风",
        "麦克风打开失败",
        "麦克风被占用",
        "麦克风授权被拒绝",
        "尚未获得麦克风授权",
    ];
    DEVICE_REASONS.contains(&reason)
}

enum VoiceRound {
    Reported {
        spoke: String,
        pauses: String,
        rhythm: String,
        note: String,
    },
    Degraded {
        reason: String,
        message: String,
    },
    Errored {
        message: String,
    },
    TimedOut,
}

/// 失败路径：语音不可用时切到打字模式并**说出具体原因**。
///
/// 本会话的 `YUNJIAN_MODEL_DIR` 指着一个空目录，所以降级是被 harness 造出来的一个
/// 真实条件，而不是等它碰巧发生。判据有三条：原因短标签在十个已定义原因码之内、
/// 后面跟着一句针对该原因的具体说明、以及打字那一路真的接上了。
fn voice_degradation(driver: &Session) -> Outcome {
    /// `app/src/contracts/voice.ts` 里 `DEGRADE_REASON_LABEL` 的十个短标签。
    /// 写死在这里是为了让「界面显示了一个表外的原因」变成一条 FAIL 而不是静默通过。
    const REASON_LABELS: [&str; 10] = [
        "本版本未编译语音",
        "系统版本过低",
        "麦克风授权被拒绝",
        "麦克风被策略禁用",
        "尚未获得麦克风授权",
        "没有可用的麦克风",
        "语音模型未就绪",
        "麦克风被占用",
        "这一次录音未被识别接受",
        "麦克风打开失败",
    ];

    let observed = (|| -> Result<Degradation> {
        enter_voice_mode(driver)?;
        match voice_phase(driver)? {
            VoicePhase::Ready => Ok(Degradation::StillAvailable),
            VoicePhase::Degraded { reason, message } => Ok(Degradation::Degraded {
                reason,
                message,
                handoff: driver.text("[data-testid='voice-typed-handoff']").ok(),
                typing_box: driver.find("[data-testid='recite-answer']").is_ok(),
            }),
        }
    })();

    match observed {
        Ok(Degradation::StillAvailable) => Outcome::not_executed(
            "本会话把 `YUNJIAN_MODEL_DIR` 指到一个空目录，本意是造出「语音模型未就绪」，\
             但可用性探测仍报语音可用，于是没有降级可观测；\
             把一个未发生的降级判成 FAIL 会是一个假故障",
            "一个能让语音可用性探测确定失败的会话（空模型目录、无输入设备或未编译语音特性）",
        ),
        Ok(Degradation::Degraded {
            reason,
            message,
            handoff,
            typing_box,
        }) => {
            let known = REASON_LABELS.contains(&reason.as_str());
            let explained = message.len() > reason.len() + 4;
            let handed_off = handoff.is_some_and(|text| !text.trim().is_empty());
            let mut missing = Vec::new();
            if !known {
                missing.push(format!(
                    "原因短标签「{reason}」不在契约定义的十个原因码之内"
                ));
            }
            if !explained {
                missing.push("原因后面没有跟一句具体说明".to_owned());
            }
            if !handed_off {
                missing.push("`voice-typed-handoff` 未渲染，没有说明已切到打字练习".to_owned());
            }
            if !typing_box {
                missing.push("`recite-answer` 不存在，打字那一路并没有真的接上".to_owned());
            }
            if missing.is_empty() {
                Outcome::pass(format!(
                    "把模型目录指空之后语音降级，界面报出的具体原因是「{reason}」\
                     （在契约定义的十个原因码内），完整判词「{}」，\
                     并已切到打字练习（`voice-typed-handoff` 与 `recite-answer` 都在）",
                    one_line(&message)
                ))
            } else {
                Outcome::fail(format!(
                    "语音确实降级了，但：{}；实测原因=「{reason}」、判词=「{}」",
                    missing.join("；"),
                    one_line(&message)
                ))
            }
        }
        Err(error) => Outcome::fail(format!("驱动语音降级失败：{error}")),
    }
}

enum Degradation {
    StillAvailable,
    Degraded {
        reason: String,
        message: String,
        handoff: Option<String>,
        typing_box: bool,
    },
}

enum VoicePhase {
    Ready,
    Degraded { reason: String, message: String },
}

/// 进背诵页、填一个真实作品标识、选语音形态、出题。
///
/// 作品标识用随包表里那一首的 `stable_id`：背诵端点要一个真实存在的 `poem_id`，
/// 而随手编一个会得到 `recite-error`，那时的判词说的是「找不到作品」而不是语音。
fn enter_voice_mode(driver: &Session) -> Result<()> {
    const POEM_ID: &str = "1eecef3c7ae3cf0b";

    if !driver.wait_for("[data-testid='nav-recite']", MOUNT_TIMEOUT) {
        bail!("等了 {} 秒，主导航仍未挂载", MOUNT_TIMEOUT.as_secs());
    }
    driver.click("[data-testid='nav-recite']")?;
    if !driver.wait_for("[data-testid='recite-poem-id']", MOUNT_TIMEOUT) {
        bail!("背诵页未渲染");
    }
    driver.clear("[data-testid='recite-poem-id']")?;
    driver.send_keys("[data-testid='recite-poem-id']", POEM_ID)?;
    driver.click("[data-testid='mode-voice']")?;
    driver.click("[data-testid='start-session']")?;
    Ok(())
}

/// 等语音面板给出结论：可用，或降级并带一个原因。
fn voice_phase(driver: &Session) -> Result<VoicePhase> {
    let deadline = Instant::now() + VOICE_PROBE_TIMEOUT;
    loop {
        if let Ok(reason) = driver.text("[data-testid='voice-degraded-reason']") {
            return Ok(VoicePhase::Degraded {
                reason,
                message: driver
                    .text("[data-testid='voice-degraded']")
                    .unwrap_or_default(),
            });
        }
        if driver.find("[data-testid='voice-record']").is_ok() {
            return Ok(VoicePhase::Ready);
        }
        if let Ok(text) = driver.text("[data-testid='recite-error']")
            && !text.trim().is_empty()
        {
            bail!("出题失败，背诵页报错：{}", one_line(&text));
        }
        if Instant::now() >= deadline {
            bail!(
                "等了 {} 秒，语音面板既没给出可用状态也没给出降级原因",
                VOICE_PROBE_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// 在结果行里按标题找到那一首并点开，返回是否找到。
fn open_row_titled(driver: &Session, title: &str) -> Result<bool> {
    if !driver.wait_for(RESULT_ROW, SEARCH_TIMEOUT) {
        return Ok(false);
    }
    let rows = driver.find_all(RESULT_ROW)?;
    for index in 0..rows.len() {
        // 每次重新取一遍元素引用：React 重渲染会让旧引用变成 stale element，
        // 而那个错误读起来像「行不见了」。
        let titles = driver.find_all(&format!("{RESULT_ROW} {ROW_TITLE}"))?;
        let Some(element) = titles.get(index) else {
            break;
        };
        if driver.element_text(element)?.trim() != title {
            continue;
        }
        let buttons = driver.find_all(&format!("{RESULT_ROW} {ROW_OPEN}"))?;
        let Some(button) = buttons.get(index) else {
            break;
        };
        driver.click_element(button)?;
        return Ok(true);
    }
    Ok(false)
}

/// 记一条断言，并**先把它引用的那张截图真的写出来**。
///
/// 截图失败不改 verdict：一条断言的成立与否与 harness 能不能存图无关，
/// 把存图失败判成产品失败会是一个假故障。但报告里那一列不能指向一个不存在的文件，
/// 所以存不下来时就不写路径。
fn record(
    driver: &Session,
    collector: &mut Collector,
    id: &str,
    outcome: Outcome,
    shots: &Path,
    file: &str,
) -> Result<()> {
    record_focused(driver, collector, id, outcome, shots, file, None)
}

/// 同 [`record`]，但当 `focus` 给了一个选择器时只截那个元素。
///
/// 判词落在折叠线以下的断言必须用它：整屏截图会拍到前置条件成立而拍不到那句判词，
/// 于是报告里那一列指向的是一张证明不了裁决的图。
fn record_focused(
    driver: &Session,
    collector: &mut Collector,
    id: &str,
    outcome: Outcome,
    shots: &Path,
    file: &str,
    focus: Option<&str>,
) -> Result<()> {
    let path = shots.join(file);
    // 聚焦截图**先试、并校验它真的拍到了东西**，拍不到就退回整屏。
    //
    // 本机实测 WebKitGTK 的元素截图端点返回一块纯黑矩形（418 字节的 PNG），
    // 它「成功」了却什么都没证明——比整屏截图更差。所以这里不能只看 `is_ok()`：
    // 那正是「退出码 0 不等于验收通过」在截图上的同一个形态。
    let captured = match focus {
        Some(css) => driver
            .screenshot_element(css, &path)
            .and_then(|()| {
                if screenshot_has_content(&path) {
                    Ok(())
                } else {
                    anyhow::bail!("元素截图是一块纯色，没有拍到内容")
                }
            })
            .or_else(|_| driver.screenshot(&path)),
        None => driver.screenshot(&path),
    };
    let shot = captured
        .is_ok()
        .then(|| format!("{}/{file}", super::linux::SHOT_DIR));
    collector.record(
        id,
        outcome.verdict,
        outcome.detail,
        outcome.executable_when,
        shot,
    )
}

/// 把多行文本压成一行，好写进报告表格的一个单元格里。
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, chars: usize) -> String {
    let flat = one_line(text);
    let taken: String = flat.chars().take(chars).collect();
    if flat.chars().count() > chars {
        format!("{taken}…")
    } else {
        taken
    }
}

/// 建一个只在本次验收里存在的临时 `HOME`。
///
/// # Errors
///
/// 建目录失败。
pub(crate) fn temp_home(root: &Path, name: &str) -> Result<PathBuf> {
    let dir = root.join("target/acceptance").join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("清理上一次的 {} 失败", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    Ok(dir)
}

/// 建一个空的模型目录，用来把语音逼进「模型未就绪」。
///
/// # Errors
///
/// 建目录失败。
pub(crate) fn empty_model_dir(root: &Path) -> Result<PathBuf> {
    let dir = root.join("target/acceptance/no-models");
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    Ok(dir)
}

/// 起一次会话并驱动它，会话建不起来时把 `ids` 全部记成未执行并写明握手结果。
///
/// # Errors
///
/// harness 自身无法启动 driver，或记录了一个未声明的断言 id。
pub(crate) fn with_session(
    app: &Path,
    env: &SessionEnv,
    ids: &[&str],
    collector: &mut Collector,
    drive: impl FnOnce(&Session, &mut Collector) -> Result<()>,
) -> Result<bool> {
    // 上一条会话的应用必须先退干净：设备与端口都是独占资源，交接不彻底会让下一条
    // 会话得出一个关于产品的错误结论。
    if !super::wait_for_app_quiesce(QUIESCE_TIMEOUT) {
        emit(&format!(
            "  上一条会话仍有 {} 个应用进程未退出，继续可能得出错误结论",
            super::running_app_pids().len()
        ));
    }

    let mut pairs = env.to_pairs();
    let owned: Vec<(&str, String)> = env
        .extra_pairs()
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    pairs.extend(owned);
    match webdriver::connect(app, &pairs)? {
        Ok(session) => {
            drive(&session, collector)?;
            Ok(true)
        }
        Err(failure) => {
            for id in ids {
                collector.record(
                    id,
                    Verdict::NotExecuted,
                    format!(
                        "WebDriver 会话未能建立，DOM 层事实无法观测；\
                         刻意不用 mock 或 stub 顶替。握手探测结果：{}",
                        failure.detail
                    ),
                    Some(
                        "`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话"
                            .to_owned(),
                    ),
                    None,
                )?;
            }
            Ok(false)
        }
    }
}

/// 一张 PNG 里是不是真的有内容，而不是一块纯色。
///
/// 判据复用 OS 通道那道绘制门的 [`super::screenshot::Paint::painted`]：主色占比 < 95%
/// 且至少三个占比达 1% 的颜色区域。**刻意不另写一套阈值**——「这张图有没有内容」
/// 只该有一个判据，两套会在某天给出互相矛盾的结论。
///
/// 读不出来时返回 `false`（当作没拍到），因为这一路的作用是决定要不要退回整屏截图，
/// 而在「不确定」时退回整屏永远不会更差。
fn screenshot_has_content(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    let Some(size) = reader.output_buffer_size() else {
        return false;
    };
    let mut buffer = vec![0; size];
    let Ok(info) = reader.next_frame(&mut buffer) else {
        return false;
    };
    // `Paint::measure` 按 4 字节步进读 RGBA。元素截图是 RGB 或 RGBA，前者要先补齐，
    // 否则会把相邻像素的通道混起来读，得出一个与图无关的直方图。
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buffer[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 0xFF])
            .collect(),
        _ => return false,
    };
    super::screenshot::Paint::measure(&rgba).painted()
}
