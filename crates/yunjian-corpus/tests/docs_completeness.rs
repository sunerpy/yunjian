//! 文档完整性门禁：清单一变、文档必须跟着变。
//!
//! # 这三条断言为什么必须存在
//!
//! 语料与模型的可辩护性不在代码里，在三份**清单**里：`corpus/DENYLIST.md` 记为什么拒，
//! `corpus/sources.toml` 与 `models.toml` 记用了谁、按什么许可。文档若与清单脱节，
//! 脱节的方向**总是**同一个：清单加了一条，文档没加——于是对外的许可陈述比实际少一项，
//! 而这**不会有任何门禁报错**。本文件把「文档覆盖清单」变成可执行断言。
//!
//! # 它为什么在 `yunjian-corpus/tests/` 而不是 `xtask/`
//!
//! `verify-sources` / `verify-models` 已经校验清单**自身**自洽（摘要、许可类别、拒绝清单
//! 不得删条目）。这里校验的是**清单与文档之间**的一致性，是一个不同的性质；放在
//! `cargo test --workspace` 会跑到的集成测试里，它就随每次 `make ci` 一起执行，
//! 而不必记得单独跑一个 xtask 子命令。
//!
//! # 三条断言各自的失败形态
//!
//! - 往 `DENYLIST.md` 加一条拒绝而不在 `docs/CORPUS.zh.md` 写理由 → 第一条红。
//! - 往 `sources.toml` 或 `models.toml` 加一个条目而不在 `LICENSES.md` 立条目 → 第二条红。
//!   （这正是本任务实测过的失败场景。）
//! - 把「不评判读音标准」从 `docs/VOICE.zh.md` 删掉或改写 → 第三条红。
//!
//! 三条都只做**存在性**判断，不判段落写得好不好——门禁能守住的是事实覆盖，
//! 守不住的是行文质量，把后者写进断言只会得到一条脆弱且无意义的测试。

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 仓库根。本 crate 的 manifest 目录是 `<root>/crates/yunjian-corpus`。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根应当存在")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("读不到 {}：{err}", path.display()))
}

// ---------------------------------------------------------------------------
// 一、`docs/CORPUS.zh.md` 必须提到 `corpus/DENYLIST.md` 的每一条
// ---------------------------------------------------------------------------

/// 解析 `DENYLIST.md` 的 `## 拒绝清单` 一节，取出每个反引号包裹的标识符。
///
/// 只认那一节里的列表项，与 `xtask/src/verify_sources.rs` 的解析口径一致——
/// 「被拒绝后的替代方案」一节里也有反引号包裹的仓库名，那些**不是**拒绝条目，
/// 全文扫描会把它们一起算进来，得到一条更严格但含义错误的断言。
fn denylist_identifiers(markdown: &str) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut inside = false;
    for line in markdown.lines() {
        if line.starts_with("## ") {
            inside = line.trim() == "## 拒绝清单";
            continue;
        }
        if !inside {
            continue;
        }
        let Some(rest) = line.strip_prefix("- `") else {
            continue;
        };
        let Some(identifier) = rest.split('`').next() else {
            continue;
        };
        if !identifier.is_empty() {
            identifiers.push(identifier.to_owned());
        }
    }
    identifiers
}

#[test]
fn corpus_doc_names_every_denylist_entry() {
    let denylist = read("corpus/DENYLIST.md");
    let identifiers = denylist_identifiers(&denylist);

    assert!(
        identifiers.len() >= 17,
        "拒绝清单解析出 {} 条，少于落地时的 17 条——要么清单被删了条目，\
         要么本测试的解析口径与 DENYLIST.md 的格式脱节了",
        identifiers.len()
    );

    let doc = read("docs/CORPUS.zh.md");
    let missing: Vec<&String> = identifiers
        .iter()
        .filter(|identifier| !doc.contains(identifier.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs/CORPUS.zh.md 没有提到这些被拒绝的数据源：{missing:?}\n\
         排除理由是语料站得住脚的记录，新增一条拒绝就必须同时写进文档。"
    );
}

// ---------------------------------------------------------------------------
// 二、`LICENSES.md` 必须为 `models.toml` 的每一行与 `sources.toml` 的每个来源立条目
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ModelManifest {
    #[serde(rename = "model")]
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    name: String,
    license: String,
}

#[derive(Debug, Deserialize)]
struct SourceManifest {
    #[serde(rename = "source")]
    sources: Vec<SourceEntry>,
}

#[derive(Debug, Deserialize)]
struct SourceEntry {
    name: String,
    git_rev: String,
    license: String,
}

#[test]
fn licenses_doc_has_an_entry_for_every_model_and_every_source() {
    let models: ModelManifest =
        toml::from_str(&read("models.toml")).expect("models.toml 应当可解析");
    let sources: SourceManifest =
        toml::from_str(&read("corpus/sources.toml")).expect("corpus/sources.toml 应当可解析");
    let doc = read("LICENSES.md");

    assert!(
        !models.models.is_empty() && !sources.sources.is_empty(),
        "两份清单都不该为空——空清单会让本断言退化成永真"
    );

    let mut missing = Vec::new();

    for model in &models.models {
        if !doc.contains(&model.name) {
            missing.push(format!("模型 `{}`（缺发布包名）", model.name));
        }
        // 许可字符串必须在文档里出现，否则「有条目」可能只是提了名字没写许可。
        if !doc.contains(&model.license) {
            missing.push(format!(
                "模型 `{}` 的许可 `{}`（文档里找不到这个 SPDX）",
                model.name, model.license
            ));
        }
    }

    for source in &sources.sources {
        if !doc.contains(&source.name) {
            missing.push(format!("语料源 `{}`（缺来源名）", source.name));
        }
        // 锁定的 revision 也要在文档里，因为「用了谁」不等于「用了它的哪一版」。
        if !doc.contains(&source.git_rev) {
            missing.push(format!(
                "语料源 `{}` 的锁定 revision `{}`",
                source.name, source.git_rev
            ));
        }
        if !doc.contains(&source.license) {
            missing.push(format!(
                "语料源 `{}` 的许可 `{}`",
                source.name, source.license
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "LICENSES.md 缺少以下条目：\n  {}\n\
         署名义务不是可选项：清单里加一个模型或来源，就必须同时在 LICENSES.md 立条目，\
         写明它的许可与（来源的）锁定 revision。",
        missing.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// 三、`docs/VOICE.zh.md` 必须含「不评判读音标准」的声明
// ---------------------------------------------------------------------------

/// 这条声明的字面形态。产品不做发音标准评分，是方案的硬性禁令之一
/// （见 `crates/yunjian-recite` 的类型边界与 compile-fail 用例），
/// 而**文档里不写等于对外没有这条承诺**。
const NO_PRONUNCIATION_SCORING: &str = "不评判读音标准";

#[test]
fn voice_doc_states_that_pronunciation_standard_is_not_assessed() {
    let doc = read("docs/VOICE.zh.md");

    assert!(
        doc.contains(NO_PRONUNCIATION_SCORING),
        "docs/VOICE.zh.md 必须含「{NO_PRONUNCIATION_SCORING}」这句声明。\
         它由 yunjian-recite 的类型边界与两条 compile-fail 用例强制，\
         但只有文档写出来，它才是一条对外承诺。"
    );

    // 声明必须**同时**点明产品不产出哪几样东西，否则「不评判读音标准」可以被读成
    // 一句立场表态而不是一条能力边界。这里判具体名词的出现，不判它周围的行文——
    // 按行判否定语境会被换行折断（实测：`声韵分` 与其否定动词分处两行），
    // 得到一条随排版变红的脆弱断言。
    for required in ["声韵分", "调型分", "guided_practice", "coverage_advisory"] {
        assert!(
            doc.contains(required),
            "docs/VOICE.zh.md 必须点名 `{required}`：\
             不评判读音标准这条声明要说清它排除了什么、v1 的取值域是什么，\
             否则它只是一句立场表态而不是能力边界。"
        );
    }

    // 反向：`scoring_mode` 绝不允许出现被撤销的两个取值。
    for withdrawn in ["\"full\"", "completeness_only"] {
        let claimed = doc.contains(withdrawn) && !doc.contains("不再含");
        assert!(
            !claimed,
            "docs/VOICE.zh.md 出现了已撤销的 scoring_mode 取值 `{withdrawn}` \
             而没有说明它已被撤销"
        );
    }
}

// ---------------------------------------------------------------------------
// 四、两份 README 的实现状态必须与代码事实一致
// ---------------------------------------------------------------------------

/// 一项功能在两份 README 里的名字，以及判定它是否已落地的**代码见证**。
///
/// 见证必须是产品代码里的路径，不能是文档或计划文件：文档与文档对照永远自洽，
/// 而漂移恰恰是「代码有了、文档没跟上」（实测：AI、背诵、voice、桌面端四项落地后
/// README 仍把它们写成一行产品代码都没有）与它的反向（把没做的写成做了）。
struct FeatureWitness {
    zh: &'static str,
    en: &'static str,
    witnesses: &'static [&'static str],
}

const FEATURES: &[FeatureWitness] = &[
    FeatureWitness {
        zh: "核心检索",
        en: "Core search",
        witnesses: &["crates/yunjian-core/src/search/topic.rs"],
    },
    FeatureWitness {
        zh: "命令行",
        en: "Command line",
        witnesses: &["crates/yunjian-cli/src/main.rs"],
    },
    FeatureWitness {
        zh: "MCP 服务器",
        en: "MCP server",
        witnesses: &["crates/yunjian-mcp/src/lib.rs"],
    },
    FeatureWitness {
        zh: "AI 赏析",
        en: "AI appreciation",
        witnesses: &[
            "crates/yunjian-ai/src/provider.rs",
            "crates/yunjian-ai/src/keystore.rs",
            "crates/yunjian-ai/src/cache.rs",
            "crates/yunjian-ai/src/stream.rs",
        ],
    },
    FeatureWitness {
        zh: "背诵训练",
        en: "Recitation practice",
        witnesses: &[
            "crates/yunjian-recite/src/align.rs",
            "crates/yunjian-recite/src/modes.rs",
            "crates/yunjian-recite/src/schedule.rs",
            "crates/yunjian-recite/src/score.rs",
        ],
    },
    FeatureWitness {
        zh: "朗读与识别",
        en: "Read-aloud and ASR",
        witnesses: &[
            "crates/yunjian-voice/src/tts.rs",
            "crates/yunjian-voice/src/recognize.rs",
            "crates/yunjian-voice/src/capture.rs",
            "crates/yunjian-voice/src/lexicon.rs",
        ],
    },
    FeatureWitness {
        zh: "桌面端",
        en: "Desktop app",
        witnesses: &[
            "crates/yunjian-app/src/ipc.rs",
            "app/src/App.tsx",
            "app/src/recite/ReciteScreen.tsx",
            "app/src/settings/KeyStoragePanel.tsx",
        ],
    },
    FeatureWitness {
        zh: "移动端",
        en: "The mobile app",
        witnesses: &["crates/yunjian-mobile/Cargo.toml"],
    },
];

/// 取 `## <标题>` 到下一个二级标题之间的正文。
fn section(markdown: &str, heading: &str) -> String {
    let start = markdown
        .find(heading)
        .unwrap_or_else(|| panic!("文档里找不到标题 `{heading}`"));
    let rest = &markdown[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// 已实现表的首列。表头由 `skip(1)` 丢掉，分隔行按字符集合识别。
fn table_first_column(part: &str) -> Vec<String> {
    part.lines()
        .filter(|line| line.starts_with('|'))
        .filter(|line| !line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
        .filter_map(|line| line.split('|').nth(1))
        .map(|cell| cell.trim().to_owned())
        .skip(1)
        .collect()
}

/// 未实现清单里每条 `- **标题**` 的标题。
///
/// 判**整个加粗标题**而不是子串，否则「桌面端」会被「桌面端真机验收」这条同时命中，
/// 于是「已实现的不得同时出现在未实现清单里」这半条断言永远失败。
fn bold_bullet_labels(part: &str) -> Vec<String> {
    part.lines()
        .filter_map(|line| line.trim_start().strip_prefix("- **"))
        .filter_map(|rest| rest.split("**").next())
        .map(|label| label.trim().to_owned())
        .collect()
}

fn assert_status_matches_code(
    path: &str,
    heading: &str,
    pending_marker: &str,
    name_of: fn(&FeatureWitness) -> &'static str,
) {
    let markdown = read(path);
    let status = section(&markdown, heading);
    let split = status.find(pending_marker).unwrap_or_else(|| {
        panic!("{path} 的「{heading}」一节里找不到未实现清单的起始标记 `{pending_marker}`")
    });
    let shipped_table = table_first_column(&status[..split]);
    let pending_list = bold_bullet_labels(&status[split..]);

    assert!(
        shipped_table.len() >= 10 && !pending_list.is_empty(),
        "{path} 的状态一节解析出 {} 行实现表、{} 条未实现条目——\
         本测试的解析口径与文档格式脱节了，再断言下去只会得到一条永真的测试",
        shipped_table.len(),
        pending_list.len()
    );

    for feature in FEATURES {
        let name = name_of(feature);
        let missing: Vec<&&str> = feature
            .witnesses
            .iter()
            .filter(|witness| !repo_root().join(witness).exists())
            .collect();
        let shipped = missing.is_empty();
        let claimed_shipped = shipped_table.iter().any(|row| row == name);
        let claimed_pending = pending_list.iter().any(|row| row == name);

        assert!(
            claimed_shipped ^ claimed_pending,
            "{path} 里「{name}」既不在实现表里、也不是未实现清单里的一条\
             （或者两处都写了）。每一项功能都必须**恰好**出现在其中一处，\
             否则读者无从判断它到底做没做。",
        );
        assert_eq!(
            shipped, claimed_shipped,
            "{path} 里「{name}」的状态与代码不符：代码见证 {:?}，缺失 {missing:?}。\
             见证齐备就必须列进实现表，缺一个就必须留在未实现清单里——\
             文档漂移应当在这里变红，而不是等人读出来。",
            feature.witnesses,
        );
    }
}

#[test]
fn readme_status_matches_code_facts() {
    assert_status_matches_code("README.md", "## 项目状态", "**尚未实现", |feature| {
        feature.zh
    });
}

#[test]
fn english_readme_status_matches_code_facts() {
    assert_status_matches_code(
        "docs/readme/README.en.md",
        "## Project status",
        "**Not implemented",
        |feature| feature.en,
    );
}

// ---------------------------------------------------------------------------
// 五、两份 README 的行数上限
// ---------------------------------------------------------------------------

/// 方案给根 README 定的硬门槛。英文镜像沿用同一个数，因为两份是同一份内容的两种语言，
/// 允许其中一份更长就等于允许它长期漂移。
const README_LINE_CEILING: usize = 230;

/// 计行数**只数真实文件的换行**，不读任何记录值。
///
/// 这条与上面那条 README 断言同源：见证必须是被约束的那个对象本身。一个记录在文档或
/// 常量里的「当前行数」与真实文件对照不上时，门禁会照记录值判绿——**实测过一次**：
/// 方案里写着 `<= 230`，没有任何断言扫过真实文件，于是根 README 漂到 **339 行**
/// （超 109 行）而没有任何门禁报错，直到人工审计才发现。
fn line_count(relative: &str) -> usize {
    read(relative).lines().count()
}

#[test]
fn both_readmes_stay_within_the_line_ceiling() {
    // 上限本身不许被悄悄放宽：它是方案的硬门槛，改它要改方案。
    assert_eq!(
        README_LINE_CEILING, 230,
        "README 行数上限由方案冻结为 230；要调整先改方案，不要改这条断言"
    );

    let mut over = Vec::new();
    for path in ["README.md", "docs/readme/README.en.md"] {
        let lines = line_count(path);
        // 下界同样要判：一份被清空或截断的 README 也是漂移，只是方向相反，
        // 而只判上限的断言会把它读成「非常合规」。
        assert!(
            lines >= 120,
            "{path} 只有 {lines} 行——README 至少要装得下状态表、快速开始与输出契约，\
             这个长度说明它被截断或清空了，而只判上限的断言会把这种情况读成合规"
        );
        if lines > README_LINE_CEILING {
            over.push(format!(
                "{path}：{lines} 行（超 {} 行）",
                lines - README_LINE_CEILING
            ));
        }
    }

    assert!(
        over.is_empty(),
        "README 超出 {README_LINE_CEILING} 行上限：\n  {}\n\
         超出的内容往 `docs/` 放，README 只留导航与快速上手。\
         **不要靠删掉「尚未实现」的条目来省行数**——那是反向漂移，\
         会让读者以为没做的事已经做了，而上面那两条状态断言正是为此存在的。",
        over.join("\n  ")
    );
}
