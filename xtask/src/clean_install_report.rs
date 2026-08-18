//! `xtask clean-install-report`：把容器里的观测行裁决成净机验收报告。
//!
//! # 为什么断言集定义在宿主而观测在容器
//!
//! 断言集是一份**预声明**清单：跑之前就固定了要验哪些事，跑完只能给每条填一个裁决，
//! 不能因为某条不好验就把它从报告里删掉。定义放在这里、观测放在容器里，是为了让
//! 「少验了一条」变成一个**硬错误**——容器少交一行，报告就生成不出来。
//!
//! 反过来，裁决逻辑不能放进容器：净机上没有 jq 也没有 Rust，而「谁来判 PASS」
//! 不该取决于容器里恰好装了什么。
//!
//! # `all_pass` 的语义
//!
//! 沿用 todo 67：**零 FAIL 且零 NOT EXECUTED**。任何一条未执行都让它变 `false`，
//! 并在报告顶部显著列出未执行项。它**不能**被读成「这个产品在所有环境上都过了」。
//!
//! # 净机必须**自带**下载器，而不是被我们装上下载器
//!
//! 2026-08-17 实测：`ubuntu:24.04`（digest `sha256:561618e2…`，与 2026-08-14 那份报告
//! 记录的**同一个**）里 curl、wget、ca-certificates **都不在场**，于是 `install.sh` 在
//! `detect_downloader` 就中止（「需要 curl 或 wget 之一来下载发布产物」），
//! `install_script_installs` FAIL，其后十条连锁 NOT EXECUTED。
//!
//! **不能在容器里 `apt-get install curl` 绕过**：那让掉的正是「净」这个性质 ——
//! 一个被我们改造过的容器验不了「用户在一台干净机器上按 README 装得上」。
//! 同理也不能把 `install.sh` 改成自带下载器：产品对用户环境的要求是它的一部分，
//! 为了让验收变绿而放宽要求等于把门禁谈掉。
//!
//! 出路因此是**换一个自带下载器的净镜像**。2026-08-18 逐个实测：
//!
//! | 镜像 | 自带下载器 | libc |
//! | --- | --- | --- |
//! | `ubuntu:24.04` | 无 | glibc |
//! | `debian:12` | 无 | glibc |
//! | `fedora:41` | `curl` | glibc 2.40 |
//! | `alpine:3.20` | BusyBox `wget` | musl |
//!
//! 于是 `CLEAN_INSTALL_IMAGE` 变成可覆盖的，并在两个镜像上各跑一遍：`fedora:41` 验
//! 「静态 musl 产物落在 glibc 发行版上跑得起来」，`alpine:3.20` 验「落在 musl 发行版上
//! 跑得起来」。**发布矩阵里 Linux 只有 musl 两条腿**（见 `release-please.yml`），
//! 所以本地镜像也只放 musl 归档——放一份线上不存在的 gnu 归档会让验收验的不是发布产物。
//! `bundled_downloader` 是报告里的一个显式字段：净机自带什么下载器必须写下来，
//! 否则「这次为什么能装上」下一次就无从复现。
//!
//! `provider_zero_calls_for_shipped_poem` 的每个分支另配了一组只吃合成计数的注入测试
//! （见本文件末尾）：判据的每一次改动都必须有人证明它真的会红，而这件事不该依赖一次要
//! docker + 本地镜像 + 完整安装的端到端跑。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::verify_sources::emit;

/// 报告 schema 版本。
const SCHEMA_VERSION: u32 = 1;

/// 一条预声明断言。
struct Declared {
    id: &'static str,
    what: &'static str,
    phase: Phase,
}

/// 断言在哪一段执行。分两段不是流程装饰：离线段必须在一个**真的没有网络**的容器里跑，
/// 而联网段必须能下载——同一个容器不可能两者都是。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    /// 联网容器：安装、取数与全流程。
    Online,
    /// 断网容器（`--network none`）：只跑字典命令。
    Offline,
    /// 宿主侧：provider 调用计数与工件校验。
    Host,
}

/// 预声明断言集。**顺序即报告顺序**；增删一条就是一次显式改动。
const DECLARED: &[Declared] = &[
    Declared {
        id: "corpus_artifact_checksum",
        what: "语料工件的 SHA-256 与随附 `.sha256` 一致（`sha256sum -c`）",
        phase: Phase::Host,
    },
    Declared {
        id: "seed_artifact_checksum",
        what: "赏析种子的 SHA-256 与随附 `.sha256` 一致",
        phase: Phase::Host,
    },
    Declared {
        id: "assets_manifest_accepted_by_app_parser",
        what: "统一资产清单被应用运行期那个解析器接受（`AssetsManifest::parse`）",
        phase: Phase::Host,
    },
    Declared {
        id: "install_script_installs",
        what: "净机上 `install.sh` 校验摘要后装出可执行的 `yunjian`",
        phase: Phase::Online,
    },
    Declared {
        id: "search_before_fetch_exits_3",
        what: "未取语料就 `search` 退出 3（而非 1）且消息指名 `corpus fetch`",
        phase: Phase::Online,
    },
    Declared {
        id: "corpus_fetch_downloads_both",
        what: "`corpus fetch` 下载并校验语料与赏析种子两件工件后退出 0",
        phase: Phase::Online,
    },
    Declared {
        id: "assets_status_reports_both",
        what: "`assets status --json` 报出语料版本、种子版本与记录数",
        phase: Phase::Online,
    },
    Declared {
        id: "search_returns_results",
        what: "`search 明月` 返回结果并退出 0",
        phase: Phase::Online,
    },
    Declared {
        id: "recite_scores_round",
        what: "`recite <id> --mode cloze` 对一轮打字作答给出评分并退出 0",
        phase: Phase::Online,
    },
    Declared {
        id: "mcp_handshake_and_tools_list",
        what: "`yunjian mcp` 完成 `initialize` 握手并回应 `tools/list`",
        phase: Phase::Online,
    },
    Declared {
        id: "shipped_hit_without_key",
        what: "**没有配置任何 API key** 时，随包集里的作品返回 `source=shipped`",
        phase: Phase::Online,
    },
    Declared {
        id: "cold_poem_without_key_asks_for_config",
        what: "没有 key 时，随包集外的作品如实返回 `configuration_required` 并给出设置路径",
        phase: Phase::Online,
    },
    Declared {
        id: "offline_no_network_proved",
        what: "对照实验证明断网容器确实无网络（向宿主镜像的 TCP 连接失败）",
        phase: Phase::Offline,
    },
    Declared {
        id: "offline_dictionary_commands",
        what: "断网下 `search` / `show` / `author` / `rhyme` / `corpus status` 全部退出 0",
        phase: Phase::Offline,
    },
    Declared {
        id: "provider_zero_calls_for_shipped_poem",
        what: "provider 调用计数器确认随包集的作品发生 **0** 次模型调用",
        phase: Phase::Host,
    },
    Declared {
        id: "provider_one_call_for_cold_poem",
        what: "provider 调用计数器确认冷诗发生**恰好 1** 次模型调用（重复请求后仍是 1）",
        phase: Phase::Host,
    },
    Declared {
        id: "shipped_dataset_is_model_output",
        what: "随包数据集的正文是开放权重模型的真实输出（`generation_executed=true`）",
        phase: Phase::Host,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Verdict {
    Pass,
    Fail,
    #[serde(rename = "NOT EXECUTED")]
    NotExecuted,
}

impl Verdict {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "PASS" => Ok(Self::Pass),
            "FAIL" => Ok(Self::Fail),
            "NOT EXECUTED" => Ok(Self::NotExecuted),
            other => bail!("未知裁决 `{other}`；只接受 PASS | FAIL | NOT EXECUTED"),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotExecuted => "NOT EXECUTED",
        }
    }
}

#[derive(Debug, Serialize)]
struct Outcome {
    id: String,
    what: String,
    phase: Phase,
    verdict: Verdict,
    detail: String,
}

#[derive(Debug, Serialize)]
struct Environment {
    /// 净机镜像。写进报告是验收要求：「报告指明所用的净环境」。
    image: String,
    /// 镜像摘要，比 tag 更能定位到底跑的是哪一版。
    image_digest: String,
    /// 净机**自带**的下载器。`install.sh` 要求 curl 或 wget 二者有其一，而这个要求是
    /// 产品对用户环境的一部分：镜像里没有就装不上，我们也不会为了让验收变绿去装一个。
    /// 因此「这次能装上是因为镜像自带什么」必须落在报告里，否则下一次无从复现。
    bundled_downloader: String,
    /// 容器内自报的系统。
    os_release: String,
    /// 内核。
    kernel: String,
    /// 容器启动时用户主目录里的条目数；0 才叫「无任何先前状态」。
    preexisting_home_entries: u32,
    /// 断网段用的隔离手段。
    offline_isolation: String,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    date: String,
    commit_sha: String,
    app_version: String,
    environment: Environment,
    all_pass: bool,
    declared: usize,
    passed: usize,
    failed: usize,
    not_executed: usize,
    /// 语料与种子两件工件的清单，含体积与摘要。
    artifacts: Vec<Artifact>,
    /// provider 计数实测的原始数字。
    provider_calls: serde_json::Value,
    assertions: Vec<Outcome>,
}

#[derive(Debug, Serialize)]
struct Artifact {
    name: String,
    bytes: u64,
    sha256: String,
}

/// 汇总观测、裁决并写出报告。
#[allow(clippy::too_many_arguments)]
pub fn run(
    observed: Vec<PathBuf>,
    provider_calls: PathBuf,
    dataset_manifest: PathBuf,
    artifacts_dir: PathBuf,
    image: String,
    image_digest: String,
    bundled_downloader: String,
    os_release: String,
    kernel: String,
    preexisting_home_entries: u32,
    offline_isolation: String,
    out_dir: PathBuf,
    date: String,
    slug: String,
    commit_sha: String,
) -> Result<()> {
    emit("== 净机验收报告 ==");

    let mut observations: BTreeMap<String, (Verdict, String)> = BTreeMap::new();
    for path in &observed {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读观测文件 {} 失败", path.display()))?;
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.splitn(3, '\t');
            let id = parts.next().unwrap_or_default().trim();
            let verdict = parts.next().unwrap_or_default().trim();
            let detail = parts.next().unwrap_or_default().trim();
            if id.is_empty() || verdict.is_empty() {
                bail!("{}:{} 观测行缺列：`{line}`", path.display(), index + 1);
            }
            let verdict = Verdict::parse(verdict)
                .with_context(|| format!("{}:{}", path.display(), index + 1))?;
            if observations
                .insert(id.to_owned(), (verdict, detail.to_owned()))
                .is_some()
            {
                bail!("断言 `{id}` 有重复观测行；一条断言只能有一个裁决");
            }
        }
    }

    let counts: serde_json::Value = read_json(&provider_calls)?;
    let dataset: serde_json::Value = read_json(&dataset_manifest)?;

    // 宿主侧的四条断言在这里裁决，不从观测文件读：它们的依据是本机文件与实测数字，
    // 让容器转抄一遍只会多一个失真环节。
    let mut host: BTreeMap<&str, (Verdict, String)> = BTreeMap::new();

    for (id, name) in [
        ("corpus_artifact_checksum", "yunjian-corpus-0.1.0.db.gz"),
        ("seed_artifact_checksum", "appreciations.json"),
    ] {
        host.insert(id, verify_checksum(&artifacts_dir, name));
    }

    let manifest_path = artifacts_dir.join(yunjian_core::assets::ASSETS_MANIFEST_FILE_NAME);
    host.insert(
        "assets_manifest_accepted_by_app_parser",
        match std::fs::read(&manifest_path) {
            Ok(bytes) => match yunjian_core::assets::AssetsManifest::parse(&bytes) {
                Ok(manifest) => (
                    Verdict::Pass,
                    format!(
                        "`AssetsManifest::parse` 接受 {}；语料 schema {} / corpus {}，种子模板 {} / {} 条",
                        manifest_path.display(),
                        manifest.corpus.schema_version,
                        manifest.corpus.corpus_version,
                        manifest.appreciation_seed.template_version,
                        manifest.appreciation_seed.record_count
                    ),
                ),
                Err(error) => (
                    Verdict::Fail,
                    format!("应用侧解析器拒绝 {}：{error}", manifest_path.display()),
                ),
            },
            Err(error) => (
                Verdict::Fail,
                format!("读 {} 失败：{error}", manifest_path.display()),
            ),
        },
    );

    host.insert(
        "provider_zero_calls_for_shipped_poem",
        zero_call_verdict(&counts, &provider_calls),
    );

    host.insert(
        "provider_one_call_for_cold_poem",
        match (
            counts["cold_calls"].as_u64(),
            counts["cold_calls_after_repeat"].as_u64(),
        ) {
            (Some(1), Some(1)) => (
                Verdict::Pass,
                format!(
                    "冷诗 {} 首次解析 1 次调用（来源 {}），重复请求后累计仍为 1（第二次走用户缓存）",
                    counts["cold_poem"].as_str().unwrap_or("?"),
                    counts["cold_source"].as_str().unwrap_or("?")
                ),
            ),
            (Some(first), Some(repeat)) => (
                Verdict::Fail,
                format!("冷诗首次 {first} 次调用、重复后累计 {repeat} 次；期望 1 与 1"),
            ),
            _ => (
                Verdict::NotExecuted,
                format!("{} 里缺冷诗计数", provider_calls.display()),
            ),
        },
    );

    // 这一条只看清单，与上一条的调用计数**刻意分开**：随包赏析不花钱（零调用）和随包赏析
    // 有内容（模型输出）是两件事，合成一条会让它们互相顶替。
    //
    // 未执行那一支保留着，且不能改成 FAIL：数据集退回未生成状态时，判 PASS 会让占位看起来
    // 像产品内容，判 FAIL 会把「这台机器没有推理运行时」说成产品缺陷。两者都不是事实。
    host.insert(
        "shipped_dataset_is_model_output",
        if dataset["generation_executed"].as_bool() == Some(true) {
            (
                Verdict::Pass,
                format!(
                    "数据集清单 `generation_executed=true`，模型 {} / 许可 {} / 运行时 {}",
                    dataset["model"].as_str().unwrap_or("?"),
                    dataset["model_license"].as_str().unwrap_or("?"),
                    dataset["provider"].as_str().unwrap_or("?")
                ),
            )
        } else {
            (
                Verdict::NotExecuted,
                format!(
                    "数据集清单 `generation_executed=false`：{}。\
                     管线、门禁与溯源字段照常校验且 {} 条记录齐备，但每条正文是未生成标记，\
                     因此「用户看到的是模型写的赏析」这件事本次没有被执行。\
                     可执行条件：一个可达的本地开放权重运行时（MIT 或 Apache-2.0 权重），\
                     即 `xtask pregenerate --endpoint <URL>`",
                    dataset["not_executed_reason"]
                        .as_str()
                        .unwrap_or("清单未给出原因"),
                    dataset["record_count"].as_u64().unwrap_or(0)
                ),
            )
        },
    );

    let mut assertions = Vec::with_capacity(DECLARED.len());
    let mut missing = Vec::new();
    for declared in DECLARED {
        let resolved = match declared.phase {
            Phase::Host => host.remove(declared.id),
            Phase::Online | Phase::Offline => observations.remove(declared.id),
        };
        match resolved {
            Some((verdict, detail)) => assertions.push(Outcome {
                id: declared.id.to_owned(),
                what: declared.what.to_owned(),
                phase: declared.phase,
                verdict,
                detail,
            }),
            None => missing.push(declared.id),
        }
    }

    // 少一条观测就中止。这条是断言集「预声明」的执行手段：不这样，一条难验的断言
    // 只要不交观测就会从报告里消失，而报告读起来仍然完整。
    if !missing.is_empty() {
        bail!(
            "以下断言没有裁决：{}；报告拒绝生成（断言集是预声明的，不能少验）",
            missing.join("、")
        );
    }
    if !observations.is_empty() {
        let extra: Vec<_> = observations.keys().cloned().collect();
        bail!(
            "观测里有断言集之外的 id：{}；新增断言必须先进 DECLARED",
            extra.join("、")
        );
    }

    let failed = assertions
        .iter()
        .filter(|o| o.verdict == Verdict::Fail)
        .count();
    let not_executed = assertions
        .iter()
        .filter(|o| o.verdict == Verdict::NotExecuted)
        .count();
    let passed = assertions
        .iter()
        .filter(|o| o.verdict == Verdict::Pass)
        .count();

    let report = Report {
        schema_version: SCHEMA_VERSION,
        date: date.clone(),
        commit_sha,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        environment: Environment {
            image,
            image_digest,
            bundled_downloader,
            os_release,
            kernel,
            preexisting_home_entries,
            offline_isolation,
        },
        all_pass: failed == 0 && not_executed == 0,
        declared: DECLARED.len(),
        passed,
        failed,
        not_executed,
        artifacts: collect_artifacts(&artifacts_dir)?,
        provider_calls: counts,
        assertions,
    };

    std::fs::create_dir_all(&out_dir)?;
    // 同一天在多个净镜像上各跑一遍是常态（一个 glibc、一个 musl），文件名里没有镜像
    // 标识时后一次会静默覆盖前一次，两份结论只剩一份。
    let stem = if slug.is_empty() {
        format!("clean-install-{date}")
    } else {
        format!("clean-install-{date}-{slug}")
    };
    let json_path = out_dir.join(format!("{stem}.json"));
    let mut json = serde_json::to_string_pretty(&report)?;
    json.push('\n');
    std::fs::write(&json_path, &json)
        .with_context(|| format!("写 {} 失败", json_path.display()))?;
    let markdown_path = out_dir.join(format!("{stem}.md"));
    std::fs::write(&markdown_path, render_markdown(&report))
        .with_context(|| format!("写 {} 失败", markdown_path.display()))?;

    emit(&format!(
        "声明 {} 条 · PASS {} · FAIL {} · NOT EXECUTED {} · all_pass={}",
        report.declared, report.passed, report.failed, report.not_executed, report.all_pass
    ));
    emit(&format!("已写出 {}", markdown_path.display()));
    emit(&format!("已写出 {}", json_path.display()));

    // 报告本身生成成功即退出 0：**报告是产物，不是门禁**。有 FAIL 时报告要能被读到，
    // 让人看见失败在哪；用非零码替代报告只会把信息丢掉。
    Ok(())
}

/// 裁决「随包命中零次模型调用」。
///
/// **两路都要成立**：fixture 那一路证明缓存路径被读到（正文与产品内容无关，因此数据集
/// 换代后它仍是同一条确定性实验），待发布数据集那一路证明**要发出去的那份工件**在运行期
/// 也零调用。少了后者，这条 PASS 可能是靠一份永不发布的 fixture 撑起来的；少了前者，
/// 数据集一换内容这条实验就跟着漂。
///
/// **本条不替 `generation_executed` 下裁决**：那是 `shipped_dataset_is_model_output` 的事。
/// 这里只要求返回正文不是未生成标记——「表里有行」不等于「行里有赏析」，而两边都是占位
/// 标记时逐字比对**也会通过**，所以标记必须单独拦一次。
///
/// 抽成独立函数是为了能被下面那组注入测试直接喂进合成计数：真跑一次净机验收要 docker、
/// 本地镜像与一次完整安装，而这条判据的每一个分支都必须有人证明它真的会红。
fn zero_call_verdict(counts: &serde_json::Value, provider_calls: &Path) -> (Verdict, String) {
    match (
        counts["shipped_calls"].as_u64(),
        counts["released_seed_calls"].as_u64(),
    ) {
        (Some(0), Some(0))
            if counts["released_seed_source"].as_str() == Some("shipped")
                && counts["released_seed_text_matches_dataset"].as_bool() == Some(true)
                && counts["released_seed_text_has_marker"].as_bool() == Some(false) =>
        {
            (
                Verdict::Pass,
                format!(
                    "`xtask provider-calls` 两路都实测 0 次调用。\
                     fixture 那一路：随包首 {}，来源 {}，正文 `{}`（永不发布，证明的是缓存路径）。\
                     待发布数据集那一路：{} 首，来源 {}，经 `{}` 导入 {} 条后命中，\
                     正文 {} 字且与 `dataset/appreciations.json` 逐字一致、不含未生成标记，\
                     首段「{}」",
                    counts["shipped_poem"].as_str().unwrap_or("?"),
                    counts["shipped_source"].as_str().unwrap_or("?"),
                    counts["fixture_text"].as_str().unwrap_or("?"),
                    counts["released_seed_poem"].as_str().unwrap_or("?"),
                    counts["released_seed_source"].as_str().unwrap_or("?"),
                    counts["released_seed_import_path"].as_str().unwrap_or("?"),
                    counts["released_seed_record_count"].as_u64().unwrap_or(0),
                    counts["released_seed_text_chars"].as_u64().unwrap_or(0),
                    counts["released_seed_text_head"].as_str().unwrap_or("?")
                ),
            )
        }
        (Some(0), Some(0)) => (
            Verdict::Fail,
            format!(
                "两路调用次数都是 0，但待发布数据集那一路的随包命中不成立：\
                 来源 {}（期望 shipped）、与数据集逐字一致={}、含未生成标记={}。\
                 零调用若不是来自一次带真实正文的随包命中，本条没有意义",
                counts["released_seed_source"].as_str().unwrap_or("?"),
                counts["released_seed_text_matches_dataset"]
                    .as_bool()
                    .map_or("?".to_owned(), |value| value.to_string()),
                counts["released_seed_text_has_marker"]
                    .as_bool()
                    .map_or("?".to_owned(), |value| value.to_string())
            ),
        ),
        (Some(fixture), Some(released)) => (
            Verdict::Fail,
            format!(
                "随包命中发生了模型调用：fixture 那一路 {fixture} 次、\
                 待发布数据集那一路 {released} 次；两者都应为 0"
            ),
        ),
        _ => (
            Verdict::NotExecuted,
            format!(
                "{} 里缺 shipped_calls 或 released_seed_calls",
                provider_calls.display()
            ),
        ),
    }
}

fn verify_checksum(dir: &Path, name: &str) -> (Verdict, String) {
    let artifact = dir.join(name);
    let digest_file = dir.join(format!("{name}.sha256"));
    let bytes = match std::fs::read(&artifact) {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                Verdict::Fail,
                format!("读 {} 失败：{error}", artifact.display()),
            );
        }
    };
    let recorded = match std::fs::read_to_string(&digest_file) {
        Ok(text) => text
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned(),
        Err(error) => {
            return (
                Verdict::Fail,
                format!("读 {} 失败：{error}", digest_file.display()),
            );
        }
    };
    let actual = sha256_hex(&bytes);
    if actual == recorded {
        (
            Verdict::Pass,
            format!(
                "{name}（{} 字节）实测 sha256 {actual} 与 {} 记录一致",
                bytes.len(),
                digest_file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
        )
    } else {
        (
            Verdict::Fail,
            format!("{name} 实测 {actual}，`.sha256` 记录 {recorded}"),
        )
    }
}

fn collect_artifacts(dir: &Path) -> Result<Vec<Artifact>> {
    let mut artifacts = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("读 {} 失败", dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let path = dir.join(&name);
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        artifacts.push(Artifact {
            name,
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }
    Ok(artifacts)
}

fn render_markdown(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# 净机安装验收 · {}\n", report.date);

    let _ = writeln!(
        out,
        "> [!{}]",
        if report.all_pass { "NOTE" } else { "WARNING" }
    );
    let _ = writeln!(out, "> **`all_pass` = `{}`。**", report.all_pass);
    let _ = writeln!(
        out,
        "> `all_pass` 的语义是**零 FAIL 且零 NOT EXECUTED**，因此只要有任何一条未执行"
    );
    let _ = writeln!(
        out,
        "> 它就是 `false`。它**不能**被读成「这个产品哪里都跑得起来」。"
    );
    if report.not_executed > 0 {
        let _ = writeln!(out, ">");
        let _ = writeln!(out, "> **未执行 {} 条：**", report.not_executed);
        for outcome in report
            .assertions
            .iter()
            .filter(|o| o.verdict == Verdict::NotExecuted)
        {
            let _ = writeln!(out, "> - `{}` — {}", outcome.id, outcome.what);
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## 净环境\n");
    let _ = writeln!(out, "| 项 | 值 |");
    let _ = writeln!(out, "| --- | --- |");
    let _ = writeln!(out, "| 镜像 | `{}` |", report.environment.image);
    let _ = writeln!(out, "| 镜像摘要 | `{}` |", report.environment.image_digest);
    let _ = writeln!(
        out,
        "| 自带下载器 | {} |",
        report.environment.bundled_downloader
    );
    let _ = writeln!(out, "| 容器内系统 | {} |", report.environment.os_release);
    let _ = writeln!(out, "| 内核 | `{}` |", report.environment.kernel);
    let _ = writeln!(
        out,
        "| 先前状态 | 用户主目录 {} 个条目（0 = 无缓存、无模型、无语料） |",
        report.environment.preexisting_home_entries
    );
    let _ = writeln!(
        out,
        "| 断网手段 | {} |",
        report.environment.offline_isolation
    );
    let _ = writeln!(out, "| 应用版本 | `{}` |", report.app_version);
    let _ = writeln!(out, "| 提交 | `{}` |", report.commit_sha);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> [!NOTE]\n\
         > 净机是**容器**而不是全新物理机：内核与宿主共用。因此「无先前状态」「无 GPU」\n\
         > 「首启派生真的跑了一遍」这些事实在这里成立；而依赖特定发行版包管理、桌面会话\n\
         > 或真实硬件的事实不成立，本报告不对它们下结论。\n\
         >\n\
         > 容器里**没有安装任何软件包**。`install.sh` 要求机器上有 curl 或 wget 之一，\n\
         > 这个要求是产品对用户环境的一部分：为了让验收变绿而 `apt-get install curl`\n\
         > 等于把要求谈掉，之后这份报告就不再能回答「用户在一台干净机器上装得上吗」。\n\
         > 上表「自带下载器」一栏记的就是镜像**原本**有什么。\n\
         >\n\
         > 净机取的是**本地镜像**里那份待发布工件，即下面「工件清单」列出的那些摘要。\n\
         > 因此本报告的结论只对这些摘要成立：某个已发布 Release 若指向的是**别的**摘要，\n\
         > 用户下载到的就不是这里验过的东西，那件事要另行核实，本报告不代它下结论。\n"
    );

    let _ = writeln!(out, "## 汇总\n");
    let _ = writeln!(
        out,
        "声明 {} 条 · PASS {} · FAIL {} · NOT EXECUTED {}\n",
        report.declared, report.passed, report.failed, report.not_executed
    );

    let _ = writeln!(out, "## 工件清单\n");
    let _ = writeln!(out, "| 工件 | 字节 | SHA-256 |");
    let _ = writeln!(out, "| --- | --- | --- |");
    for artifact in &report.artifacts {
        let _ = writeln!(
            out,
            "| `{}` | {} | `{}` |",
            artifact.name, artifact.bytes, artifact.sha256
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## 逐条断言\n");
    let _ = writeln!(out, "| 断言 | 段 | 裁决 | 依据 |");
    let _ = writeln!(out, "| --- | --- | --- | --- |");
    for outcome in &report.assertions {
        let phase = match outcome.phase {
            Phase::Online => "联网容器",
            Phase::Offline => "断网容器",
            Phase::Host => "宿主",
        };
        let _ = writeln!(
            out,
            "| `{}`<br>{} | {} | **{}** | {} |",
            outcome.id,
            outcome.what,
            phase,
            outcome.verdict.as_str(),
            outcome.detail.replace('|', "\\|")
        );
    }
    out
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("读 {} 失败", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("解析 {} 失败", path.display()))
}

fn sha256_hex(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份实测通过的两路计数。各条测试从它派生，只改要验的那一个字段。
    fn passing_counts() -> serde_json::Value {
        serde_json::json!({
            "shipped_calls": 0,
            "cold_calls": 1,
            "cold_calls_after_repeat": 1,
            "shipped_source": "shipped",
            "cold_source": "generated",
            "shipped_text": "随包赏析 fixture 正文（验证缓存路径用，非模型输出，永不发布）",
            "shipped_poem": "00001539ed2f02b5",
            "cold_poem": "0000168330a10ebb",
            "fixture_seed": true,
            "fixture_text": "随包赏析 fixture 正文（验证缓存路径用，非模型输出，永不发布）",
            "released_seed_calls": 0,
            "released_seed_source": "shipped",
            "released_seed_poem": "062f574ab2986a9b",
            "released_seed_text_matches_dataset": true,
            "released_seed_text_has_marker": false,
            "released_seed_text_chars": 190,
            "released_seed_text_head": "《春夜喜雨》是唐代诗人杜甫的一首咏春诗。",
            "released_seed_record_count": 16,
            "released_seed_generation_executed": true,
            "released_seed_model": "deepseek-r1:7b",
            "released_seed_model_license": "MIT",
            "released_seed_import_path": "AppreciationCache::replace_shipped_seed",
        })
    }

    fn verdict_of(counts: &serde_json::Value) -> (Verdict, String) {
        zero_call_verdict(
            counts,
            Path::new("docs/reports/clean-install-provider-calls.json"),
        )
    }

    #[test]
    fn two_zero_call_paths_with_real_body_pass_and_the_detail_names_both() {
        let (verdict, detail) = verdict_of(&passing_counts());
        assert_eq!(
            verdict,
            Verdict::Pass,
            "两路都零调用且正文为真时应 PASS：{detail}"
        );
        // 判词必须同时点名两路，否则读报告的人无法分辨这条 PASS 是靠哪一路成立的。
        for needle in [
            "两路都实测 0 次调用",
            "fixture 那一路",
            "待发布数据集那一路",
            "AppreciationCache::replace_shipped_seed",
            "逐字一致、不含未生成标记",
        ] {
            assert!(detail.contains(needle), "判词缺「{needle}」：{detail}");
        }
    }

    /// **这条是本次改正的核心**：旧判据只看 `shipped_calls`，于是一份正文是占位标记的
    /// 待发布数据集也能拿到 PASS —— 「随包不花钱」成立而「随包有内容」不成立。
    #[test]
    fn a_placeholder_body_in_the_released_seed_fails_even_though_both_paths_are_zero_call() {
        let mut counts = passing_counts();
        counts["released_seed_text_has_marker"] = serde_json::json!(true);
        let (verdict, detail) = verdict_of(&counts);
        assert_eq!(
            verdict,
            Verdict::Fail,
            "待发布数据集正文是未生成标记时必须 FAIL，否则占位会拿到一条零调用 PASS：{detail}"
        );
        assert!(
            detail.contains("含未生成标记=true"),
            "判词必须点名是标记那一项不合，而不是笼统说随包命中不成立：{detail}"
        );
    }

    /// 逐字比对与标记检查**都要**：两边都是占位时它们逐字相同，比对会通过。
    #[test]
    fn the_literal_comparison_alone_would_let_a_placeholder_through() {
        let mut counts = passing_counts();
        counts["released_seed_text_matches_dataset"] = serde_json::json!(true);
        counts["released_seed_text_has_marker"] = serde_json::json!(true);
        assert_eq!(
            verdict_of(&counts).0,
            Verdict::Fail,
            "逐字一致但含标记仍须 FAIL；否则占位数据集与占位随包行互相印证成一条 PASS"
        );
    }

    #[test]
    fn a_body_that_differs_from_the_dataset_fails() {
        let mut counts = passing_counts();
        counts["released_seed_text_matches_dataset"] = serde_json::json!(false);
        let (verdict, detail) = verdict_of(&counts);
        assert_eq!(
            verdict,
            Verdict::Fail,
            "正文与数据集不同必须 FAIL：{detail}"
        );
        assert!(
            detail.contains("与数据集逐字一致=false"),
            "判词要点名是哪一项不合：{detail}"
        );
    }

    /// 零调用若不是来自随包命中就没有意义：来源退化成 generated 时那个 0 是假的。
    #[test]
    fn a_zero_that_did_not_come_from_a_shipped_hit_fails() {
        let mut counts = passing_counts();
        counts["released_seed_source"] = serde_json::json!("generated");
        let (verdict, detail) = verdict_of(&counts);
        assert_eq!(
            verdict,
            Verdict::Fail,
            "来源不是 shipped 必须 FAIL：{detail}"
        );
        assert!(
            detail.contains("期望 shipped"),
            "判词要写明期望值：{detail}"
        );
    }

    #[test]
    fn a_model_call_on_either_path_fails_and_the_detail_reports_both_numbers() {
        for (fixture, released) in [(1_u64, 0_u64), (0, 1), (2, 3)] {
            let mut counts = passing_counts();
            counts["shipped_calls"] = serde_json::json!(fixture);
            counts["released_seed_calls"] = serde_json::json!(released);
            let (verdict, detail) = verdict_of(&counts);
            assert_eq!(
                verdict,
                Verdict::Fail,
                "fixture={fixture} released={released} 时必须 FAIL：{detail}"
            );
            assert!(
                detail.contains(&format!("fixture 那一路 {fixture} 次"))
                    && detail.contains(&format!("待发布数据集那一路 {released} 次")),
                "判词必须同时报两路的次数，否则看不出是哪一路花了钱：{detail}"
            );
        }
    }

    /// 缺键记 NOT EXECUTED 而不是 FAIL：**旧版计数文件没有 `released_seed_calls`**，
    /// 把它读成 FAIL 会把「用旧工具跑的」说成「产品坏了」。
    #[test]
    fn a_counts_file_without_the_released_seed_path_is_not_executed() {
        let mut counts = passing_counts();
        let object = counts.as_object_mut().expect("合成计数应是对象");
        object.remove("released_seed_calls");
        let (verdict, detail) = verdict_of(&counts);
        assert_eq!(
            verdict,
            Verdict::NotExecuted,
            "缺 released_seed_calls 应记未执行：{detail}"
        );
        assert!(
            detail.contains("released_seed_calls"),
            "判词要点名缺的是哪个键：{detail}"
        );
    }

    /// `generation_executed` 由 `shipped_dataset_is_model_output` 单独裁决，本条不看它 ——
    /// 合成一条会让「随包不花钱」与「随包有内容」互相顶替。
    #[test]
    fn this_assertion_does_not_read_generation_executed() {
        let mut counts = passing_counts();
        counts["released_seed_generation_executed"] = serde_json::json!(false);
        assert_eq!(
            verdict_of(&counts).0,
            Verdict::Pass,
            "本条只看调用次数与正文；清单的 generation_executed 由另一条断言承担"
        );
    }

    /// 断言集里必须有且只有一条零调用断言，且它在宿主段。
    #[test]
    fn the_zero_call_assertion_is_declared_exactly_once_in_the_host_phase() {
        let matched: Vec<_> = DECLARED
            .iter()
            .filter(|declared| declared.id == "provider_zero_calls_for_shipped_poem")
            .collect();
        assert_eq!(matched.len(), 1, "零调用断言应恰好声明一条");
        assert_eq!(
            matched[0].phase,
            Phase::Host,
            "它的依据是宿主侧实测数字，不该从容器观测文件读"
        );
    }
}
