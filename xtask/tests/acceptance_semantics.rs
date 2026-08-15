//! 验收命令必须断言**语义**，不能只靠退出码。
//!
//! # 这道门禁存在的原因
//!
//! todo 75 曾被判为完成，而仓库里一个 Release、一个 tag 都没有。成因不是有人偷懒，
//! 而是那条验收 shell 形如 `gh release view … --jq '.x'`——**`gh --jq` 对 `false`
//! 与 `null` 都返回 0**，于是「发布了吗」这个问题在机械上永远答「是」。同一轮里
//! todo 20 也栽在同一形态上：`xtask corpus-measure --scale 10k` 退出 0，而机器可读
//! 结果把 10k 标成 `state=not_measured`。
//!
//! 通用教训是一句话：**退出码 0 不等于验收通过。** 凡验收命令产出机器可读结果
//! （JSON / manifest / 报告），就必须再断言那个结果里的语义字段。
//!
//! 而「通用教训」如果只写在笔记里，下一次还会犯。所以这里把它变成可机械发现的：
//! 手法沿用 README 行数守卫与打包目标守卫——**常量自锁 + 扫真实文件，不读任何
//! 记录值**。把 `.github/workflows/` 里任意一条断言改回「只看退出码」的形态，
//! 下面就有用例变红。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 允许存在的「只打印不断言」的 `gh --jq` 条数。
///
/// 自锁为 0：这不是一个可以随手放宽的预算。真有一条 `gh --jq` 只为打日志而存在，
/// 也得把它的结果同时喂给一条比较——否则那条日志就是下一次假绿的藏身处。
const BARE_GH_JQ_ALLOWANCE: usize = 0;

/// `gh release view --json` 实际支持的字段。
///
/// **实测自 `gh release view --help`（gh 2.86.0）**，不是从文档抄的。写死它是为了
/// 挡住一类只在「真正发第一版」那一刻才暴露的失败：`isLatest` 看起来理所当然
/// 存在，实际会让 `gh` 以 `Unknown JSON field` 退出 1——发布链路上最后一步炸掉，
/// 而错误信息与发布本身毫无关系。
const GH_RELEASE_VIEW_JSON_FIELDS: [&str; 18] = [
    "apiUrl",
    "assets",
    "author",
    "body",
    "createdAt",
    "databaseId",
    "id",
    "isDraft",
    "isImmutable",
    "isPrerelease",
    "name",
    "publishedAt",
    "tagName",
    "tarballUrl",
    "targetCommitish",
    "uploadUrl",
    "url",
    "zipballUrl",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("从 xtask/ 推出仓库根目录")
        .to_path_buf()
}

fn workflows() -> Vec<(String, String)> {
    let dir = repo_root().join(".github/workflows");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("读取 .github/workflows") {
        let path = entry.expect("目录项").path();
        if path.extension().is_some_and(|ext| ext == "yml") {
            let name = path
                .file_name()
                .expect("文件名")
                .to_string_lossy()
                .into_owned();
            out.push((name, std::fs::read_to_string(&path).expect("读取工作流")));
        }
    }
    out.sort();
    assert!(!out.is_empty(), "一个工作流都没扫到，路径写错了");
    out
}

/// 把行尾续行合并成一条逻辑命令。
///
/// 不合并就会把 `gh release view … \` 与下一行的 `--jq '…'` 当成两条命令，于是
/// 「这条 `--jq` 的结果有没有被比较」永远判不准——真实的假绿恰好都是多行写法。
fn logical_lines(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buffer = String::new();
    for raw in script.lines() {
        let line = raw.trim();
        if line.starts_with('#') && buffer.is_empty() {
            continue;
        }
        if let Some(head) = line.strip_suffix('\\') {
            buffer.push_str(head.trim_end());
            buffer.push(' ');
            continue;
        }
        buffer.push_str(line);
        out.push(std::mem::take(&mut buffer));
    }
    if !buffer.is_empty() {
        out.push(buffer);
    }
    out
}

/// 这条逻辑命令有没有真的用上它取到的值？
///
/// 三种算「用上了」：喂进命令替换（`$(…)`，随后必然被 `test`/`[` 比较或赋值给变量）、
/// 显式赋值给变量、重定向进文件（供后续 `jq -e` 断言）。除此之外就是只打印。
fn consumes_output(command: &str) -> bool {
    command.contains("$(") || command.contains('>') || command.contains("=$")
}

#[test]
fn no_workflow_prints_a_gh_jq_result_without_asserting_it() {
    assert_eq!(
        BARE_GH_JQ_ALLOWANCE, 0,
        "要放宽这个数先改方案，不要改这条断言"
    );
    let mut bare = Vec::new();
    for (name, body) in workflows() {
        for command in logical_lines(&body) {
            if !command.contains("--jq") || command.trim_start().starts_with('#') {
                continue;
            }
            if !consumes_output(&command) {
                bare.push(format!("{name}: {command}"));
            }
        }
    }
    assert_eq!(
        bare.len(),
        BARE_GH_JQ_ALLOWANCE,
        "这些 `gh --jq` 的结果没有被任何比较消费掉。`gh --jq` 对 `false` 与 `null` \
         都返回 0，所以它们的退出码不构成断言：\n{}",
        bare.join("\n")
    );
}

#[test]
fn workflows_only_request_json_fields_gh_actually_supports() {
    let supported: BTreeSet<&str> = GH_RELEASE_VIEW_JSON_FIELDS.iter().copied().collect();
    assert!(
        !supported.contains("isLatest"),
        "`isLatest` 不是 `gh release view --json` 的字段（实测 Unknown JSON field）；\
         把它加进这份名单只会让门禁失去意义，`latest` 要走 REST 的 releases/latest"
    );
    for (name, body) in workflows() {
        for command in logical_lines(&body) {
            let Some(rest) = command.split_once("gh release view") else {
                continue;
            };
            let Some(after) = rest.1.split_once("--json ") else {
                continue;
            };
            let fields = after
                .1
                .split_whitespace()
                .next()
                .expect("--json 后面应有字段列表");
            for field in fields.split(',') {
                assert!(
                    supported.contains(field),
                    "{name} 向 `gh release view --json` 请求了不存在的字段 `{field}`；\
                     这条命令在真正发布时才会以一个与发布无关的错误炸掉"
                );
            }
        }
    }
}

/// `| tee` 是本仓库里最常见的假绿形态：管道的退出码是 `tee` 的，左边失败照样绿。
///
/// GitHub 的 `shell: bash` 会带上 `-o pipefail`，默认 shell（`bash -e`）不会——
/// 所以每处 `| tee` 必须二者之一成立，且判据是那一步自己的配置，不是整份文件。
#[test]
fn every_tee_pipeline_in_ci_is_protected_from_losing_the_exit_code() {
    let mut checked = 0usize;
    for (name, body) in workflows() {
        for step in body.split("\n      - ") {
            if !logical_lines(step)
                .iter()
                .any(|line| line.contains("| tee"))
            {
                continue;
            }
            checked += 1;
            assert!(
                step.contains("shell: bash") || step.contains("pipefail"),
                "{name} 有一处 `| tee` 既没有 `shell: bash`（自带 -o pipefail）\
                 也没有显式 pipefail，左边命令失败时这一步仍然会绿：\n{step}"
            );
        }
    }
    assert!(checked > 0, "一处 `| tee` 都没扫到，切分步骤的方式失效了");
}

/// 发布之后必须**独立复核发布这件事本身**。
///
/// 这一条守的正是 todo 75 的洞：上传步骤退出 0 不等于 Release 存在、不等于资产
/// 完整、更不等于资产内容正确。所以复核步骤必须同时具备三件事，缺一件都不算：
/// 断言不是 draft、把资产**下载回来**、对下载物重算 SHA-256。
#[test]
fn the_corpus_release_workflow_verifies_the_release_after_publishing_it() {
    let body = std::fs::read_to_string(repo_root().join(".github/workflows/corpus-release.yml"))
        .expect("读取 corpus-release.yml");
    let publish = body.find("gh release create").expect("应有发布步骤");
    let verify = body
        .find("gh release view \"$TAG\" --json isDraft,tagName,assets")
        .expect("应有一步用受支持的字段复核 Release");
    assert!(
        verify > publish,
        "复核步骤必须在发布之后，否则它复核的是上一次发布"
    );
    let tail = &body[verify..];
    for needle in [
        "jq -e",
        ".isDraft == false",
        "gh release download",
        "sha256sum -c",
    ] {
        assert!(
            tail.contains(needle),
            "corpus-release.yml 的发布后复核缺 `{needle}`；\
             少了它这一步就退回「上传命令退出 0 即算发布成功」"
        );
    }
}

/// 一条 `jq` 引用的 JSON 文件路径。用它判断「同一份 JSON 是否另有一条 `jq -e`
/// 在断言」——只打印是允许的，只打印且**没人断言**才是假绿。
fn json_operand(command: &str) -> Option<&str> {
    command
        .split_whitespace()
        .find(|token| token.trim_matches('"').ends_with(".json"))
        .map(|token| token.trim_matches('"'))
}

/// 读文件的 `jq` 必须么用 `-e`，么把结果交给比较，么同一份 JSON 另有 `-e` 断言。
#[test]
fn jq_assertions_are_either_dash_e_or_compared() {
    let mut sources = workflows();
    sources.push((
        "Makefile".to_owned(),
        std::fs::read_to_string(repo_root().join("Makefile")).expect("读取 Makefile"),
    ));
    let mut bare = Vec::new();
    for (name, body) in sources {
        for command in logical_lines(&body) {
            let trimmed = command.trim_start().trim_start_matches(['@', '-']);
            if !trimmed.starts_with("jq ") || trimmed.starts_with("jq -e") {
                continue;
            }
            // `jq -n` 构造 JSON、`jq -r` 取值，两者都不是断言：它们的产物由命令替换
            // 或重定向接走，真正的断言在别处。
            if consumes_output(&command)
                || trimmed.starts_with("jq -n")
                || trimmed.starts_with("jq -r")
            {
                continue;
            }
            // 只打印是允许的——前提是同一份 JSON 另有一条 `jq -e` 在断言它。
            let asserted = json_operand(&command).is_some_and(|file| {
                logical_lines(&body)
                    .iter()
                    .any(|other| other.contains("jq -e") && other.contains(file))
            });
            if asserted {
                continue;
            }
            bare.push(format!("{name}: {command}"));
        }
    }
    assert!(
        bare.is_empty(),
        "这些 `jq` 既没有 `-e` 也没有把结果交给比较，`false` 会以退出码 0 通过：\n{}",
        bare.join("\n")
    );
}
