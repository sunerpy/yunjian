//! 前置工件缺失时的统一说明。
//!
//! # 这个模块为什么存在
//!
//! 有三条子命令（`corpus-package`、`pregenerate`、`provider-calls`）消费同一份
//! **gitignored** 的大工件 `corpus/build/release/corpus.db`（随包 603 MiB / 审计另 331 MiB）。
//! 新鲜检出里它不存在，而三处各自的失败形态原来并不一致：`corpus-package` 有一句
//! 「先跑 corpus-build」，另两条只把 SQLite 的
//! `unable to open database file` 原样抛出去——**读者无从判断这是环境缺工件还是代码坏了**。
//! F1 合规审计正是在隔离 worktree 里撞上后者，把 todo 21/41 判成 FAIL。
//!
//! 把说明收在一处，是为了让「缺什么、怎么补、要多久」这三件事只有一份定义。
//! 三个调用方共享同一段文字，所以它们不会再各自漂移。
//!
//! # 为什么不在这里替用户把工件建出来
//!
//! 构建要三份上游检出（约 833 MB）和约 9 分钟；下载要网络。两者都是**用户要知情的
//! 代价**，隐式代跑会把一条只读命令变成一条会占几个 GB 磁盘的命令。所以这里只报告，
//! 不代劳。

use std::path::Path;

use anyhow::{Result, bail};

/// 随包语料库的构建输出目录。三条消费方的默认路径都指向它下面的 `corpus.db`。
const BUILD_OUT_DIR: &str = "corpus/build/release";

/// 语料工件的发布 tag 形态。与应用发布 tag 分开，所以语料刷新不牵动应用发版。
const CORPUS_RELEASE_TAG_GLOB: &str = "corpus-v*";

/// 缺少随包语料库时的可执行说明。
///
/// 消费方在**打开数据库之前**调用它。放在打开之前而不是包装打开的错误，是因为
/// `Connection::open_with_flags` 对「文件不存在」与「文件在但读不了」返回同一个
/// `Error code 14`，事后无法区分——而这两种情况该给的建议完全不同。
pub fn require_corpus_db(corpus_db: &Path) -> Result<()> {
    if corpus_db.exists() {
        return Ok(());
    }
    bail!(
        "缺少前置工件：随包语料库 {} 不存在。\n\
         \n\
         它是 gitignored 的生成物（随包库约 603 MiB，同批审计库另约 331 MiB），\
         新鲜检出里不会有。两条路补齐，任选其一：\n\
         \n\
         1) 本机构建（约 9 分钟，需三份上游检出约 833 MB）：\n\
         \x20     cargo run -p xtask --release -- corpus-build \\\n\
         \x20       --chinese-poetry-dir <检出> --werneror-dir <检出> --rhyme-dir <检出> \\\n\
         \x20       --out-dir {BUILD_OUT_DIR}\n\
         \x20   三份检出按 corpus/sources.toml 的锁定 revision 取；构建是确定性的，\
         同一输入两次构建的 corpus.db SHA-256 完全相同。\n\
         \n\
         2) 取已发布工件：从 `{CORPUS_RELEASE_TAG_GLOB}` tag 的 GitHub Release 下载\n\
         \x20   yunjian-corpus-<版本>.db.gz，校验同名 .sha256 后解压到 {BUILD_OUT_DIR}/corpus.db。\n\
         \x20   注意 `pregenerate` 与 `corpus-package` 还需要同批的 corpus-audit.db，\
         它只由 corpus-build 产出，发布工件里没有。\n\
         \n\
         这不是代码缺陷，是环境缺前置工件。详见 docs/DEVELOPMENT.zh.md。",
        corpus_db.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_path_is_accepted() {
        let file = std::env::temp_dir().join(format!("yunjian-prereq-{}", std::process::id()));
        std::fs::write(&file, b"x").expect("能写临时文件");
        assert!(require_corpus_db(&file).is_ok());
        let _ = std::fs::remove_file(&file);
    }

    /// 这条断言守的是**说明本身**，不是「有没有报错」。
    ///
    /// 原状里 `pregenerate` 也报错，只是报的是 SQLite 的 `unable to open database file`——
    /// 读者从中既看不出缺的是哪个文件、也看不出怎么补，于是 F1 审计把环境缺前置读成实现缺陷。
    /// 因此逐项判：缺失路径、两条补齐途径、以及「这不是代码缺陷」这句定性。
    #[test]
    fn a_missing_path_yields_an_actionable_message() {
        let absent = std::env::temp_dir().join("yunjian-prereq-does-not-exist/corpus.db");
        let message = require_corpus_db(&absent)
            .expect_err("缺失应当报错")
            .to_string();

        for required in [
            "corpus.db",
            "gitignored",
            "corpus-build",
            CORPUS_RELEASE_TAG_GLOB,
            BUILD_OUT_DIR,
            "docs/DEVELOPMENT.zh.md",
            "不是代码缺陷",
        ] {
            assert!(
                message.contains(required),
                "前置说明缺少 `{required}`；只报「打不开数据库」会让读者以为是实现坏了：\n{message}"
            );
        }
    }
}
