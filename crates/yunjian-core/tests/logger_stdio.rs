//! stdout 纯净性契约：本工作区的日志一个字节都不许落到 stdout。
//!
//! 同一个 `yunjian` 二进制承载 MCP stdio 服务器，stdout 就是协议流本身；MCP 规范要求服务器
//! 绝不能往 stdout 写非协议内容。所以这条断言不是风格检查，而是协议正确性检查。
//!
//! 为什么是 `harness = false`：libtest 自己就会往 stdout 打印 `running 1 test` 和统计行，
//! 任何以 libtest 为外壳的子进程都不可能让 stdout 逐字节为空。这里父子进程是同一个可执行
//! 文件，靠命令行标记而非环境变量区分角色——环境变量会被后代进程继承，一个残留的变量就能
//! 让整条用例静默走进子进程分支、什么都没断言地退出 0。
//!
//! 断言全部走 `assert!`/`panic!`：panic 信息由 panic hook 写 stderr，不经过 `println!` /
//! `eprintln!`，因此本文件本身也不违反 stdout 禁令，也不会撞上后续 todo 的 `print_stderr` 门禁。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use yunjian_core::LoggerConfig;

/// 子进程角色标记，其后紧跟日志目录。cargo 传给本二进制的过滤参数不会与它重名。
const CHILD_FLAG: &str = "--yunjian-logger-child";

const FILE_PREFIX: &str = "stdio-probe";

const MARK_INFO: &str = "MARK-INFO-4f2a9c";
const MARK_WARN: &str = "MARK-WARN-4f2a9c";
const MARK_ERROR: &str = "MARK-ERROR-4f2a9c";
const MARK_DEBUG_BEFORE: &str = "MARK-DEBUG-BEFORE-4f2a9c";
const MARK_DEBUG_AFTER: &str = "MARK-DEBUG-AFTER-4f2a9c";

/// ANSI 转义序列的引子（CSI）。
const ANSI_CSI: &[u8] = b"\x1b[";

fn main() {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == CHILD_FLAG {
            let dir = args.next().expect("子进程标记之后必须跟日志目录");
            child(PathBuf::from(dir));
            return;
        }
    }
    parent();
}

/// 子进程：初始化日志，按四个级别各打一条，再在运行时把级别调到 debug。
fn child(dir: PathBuf) {
    let cfg = LoggerConfig {
        level: "info".to_owned(),
        json: false,
        dir,
        file_prefix: FILE_PREFIX.to_owned(),
    };

    let guard = yunjian_core::init_logger(&cfg).expect("子进程初始化日志");
    assert!(guard.is_some(), "可写目录下必须挂上文件层");

    tracing::debug!("{MARK_DEBUG_BEFORE}");
    tracing::info!("{MARK_INFO}");
    tracing::warn!("{MARK_WARN}");
    tracing::error!("{MARK_ERROR}");

    yunjian_core::set_log_level("debug").expect("运行时调到 debug");
    assert_eq!(yunjian_core::current_log_level(), "debug");
    tracing::debug!("{MARK_DEBUG_AFTER}");

    // 显式丢弃 guard，把非阻塞 writer 缓冲区里的记录刷进文件后再退出。
    drop(guard);
}

fn parent() {
    let dir = TempDir::new();
    let exe = std::env::current_exe().expect("取当前可执行文件路径");
    let output = Command::new(exe)
        .arg(CHILD_FLAG)
        .arg(dir.path())
        // RUST_LOG 会盖掉配置里的级别，留着它 debug 记录就会在调级之前就出现。
        .env_remove("RUST_LOG")
        .output()
        .expect("拉起子进程");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "子进程未通过自身断言（退出码 {:?}）:\n{stderr}",
        output.status.code()
    );

    assert!(
        output.stdout.is_empty(),
        "日志必须全部走 stderr：stdout 收到了 {} 字节 {:?}",
        output.stdout.len(),
        String::from_utf8_lossy(&output.stdout)
    );

    assert!(!stderr.is_empty(), "stderr 必须收到日志");
    for mark in [MARK_INFO, MARK_WARN, MARK_ERROR, MARK_DEBUG_AFTER] {
        assert!(stderr.contains(mark), "stderr 缺少 {mark}:\n{stderr}");
    }
    assert!(
        !stderr.contains(MARK_DEBUG_BEFORE),
        "info 级别下不该出现 debug 记录，说明过滤器没生效:\n{stderr}"
    );
    assert!(
        !contains_ansi(stderr.as_bytes()),
        "stderr 被重定向到管道时不该带 ANSI 转义序列:\n{stderr}"
    );

    let log = sole_log_file(dir.path());
    let bytes = std::fs::read(&log).expect("读回日志文件");
    assert!(
        !contains_ansi(&bytes),
        "日志文件里出现 ANSI 转义序列: {}",
        log.display()
    );

    let text = String::from_utf8_lossy(&bytes);
    for mark in [MARK_INFO, MARK_WARN, MARK_ERROR, MARK_DEBUG_AFTER] {
        assert!(text.contains(mark), "日志文件缺少 {mark}:\n{text}");
    }
    assert!(
        !text.contains(MARK_DEBUG_BEFORE),
        "日志文件里不该有被过滤掉的记录:\n{text}"
    );
}

fn contains_ansi(bytes: &[u8]) -> bool {
    bytes
        .windows(ANSI_CSI.len())
        .any(|window| window == ANSI_CSI)
}

/// 按天滚动只会产出一个文件（同一天内），顺带断言命名前缀符合配置。
fn sole_log_file(dir: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("列出日志目录")
        .map(|entry| entry.expect("读取目录项").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("{FILE_PREFIX}.")))
        })
        .collect();
    found.sort();
    assert_eq!(
        found.len(),
        1,
        "应当恰好有一个 `{FILE_PREFIX}.<日期>` 文件，实际 {found:?}"
    );
    found.remove(0)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间晚于 UNIX 纪元")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yunjian-logger-stdio-{}-{nanos}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("创建临时目录");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
