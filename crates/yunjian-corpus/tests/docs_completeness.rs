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
