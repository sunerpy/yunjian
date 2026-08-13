//! 安装脚本与 README 契约的门禁。
//!
//! # 为什么这三件事放在一起
//!
//! `scripts/install.sh`、`scripts/install.ps1` 与 README 的「与 LLM 协作」段落是**同一份
//! 契约的三个副本**：资产名、环境变量名、退出码、两种 MCP 客户端配置形态。三处任意一处
//! 单独改动都会让另外两处变成谎言，而这种漂移只有用户会发现——脚本装不上，或者 AI 客户端
//! 读到一个语法合法而永远连不上的条目。
//!
//! 因此这里做三类断言：
//!
//! 1. **安装脚本真的跑得通**：对着一个进程内的 mock release server 走完整条路，断言二进制
//!    落到**默认目录**且校验和被真的算过。
//! 2. **校验失败必须什么都不装**：喂一份被篡改的归档，断言退出 3 且目标目录仍然是空的。
//!    这条比 1 更重要——1 失败会被立刻发现，2 失败会静默装上一个被换过的二进制。
//! 3. **README 里那两份 JSON 都能解析且形态不同**：Claude 的 `command` 是字符串，OpenCode
//!    的是数组。只给一种就等于把一半用户送进「配了但连不上」。
//!
//! mock server 是手写的最小 HTTP：安装脚本的正确性依赖的是「404 时回退、200 时校验」这两
//! 条分支，而不是任何 HTTP 特性。为此引一个 web 框架的 dev-dependency 不成比例。

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

/// 仓库根目录。`CARGO_MANIFEST_DIR` 指向 `crates/yunjian-cli`。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根目录可解析")
}

fn read_root_readme() -> String {
    fs::read_to_string(repo_root().join("README.md")).expect("根 README 可读")
}

// ------------------------------------------------------------------ mock server

/// 一个只为本测试存在的最小 release server。
///
/// 路由按**后缀**而不是完整路径匹配：目标三元组由 `uname` 在脚本里决定，测试进程不该
/// 重新推导一遍（推导错了会得到一个「测试通过但脚本在别的机器上装不上」的假绿）。
struct MockRelease {
    port: u16,
    _handle: thread::JoinHandle<()>,
}

impl MockRelease {
    /// `routes` 的键是 URL 后缀；`deny` 里任一子串命中即回 404。
    fn start(routes: HashMap<String, Vec<u8>>, deny: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("能绑到回环端口");
        let port = listener.local_addr().expect("能读到端口").port();
        let (ready_tx, ready_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            ready_tx.send(()).expect("主线程还在等");
            // 一次安装最多取 3 个文件（探测的归档 + 命中的归档 + 摘要）。给足余量后
            // 线程自然退出，测试不必显式关服务。
            for _ in 0..16 {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                serve_one(stream, &routes, &deny);
            }
        });

        ready_rx.recv().expect("服务线程已启动");
        Self {
            port,
            _handle: handle,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn serve_one(mut stream: TcpStream, routes: &HashMap<String, Vec<u8>>, deny: &[String]) {
    let mut buf = [0_u8; 4096];
    let Ok(n) = stream.read(&mut buf) else { return };
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();

    let denied = deny.iter().any(|needle| path.contains(needle.as_str()));
    let body = if denied {
        None
    } else {
        routes
            .iter()
            .find(|(suffix, _)| path.ends_with(suffix.as_str()))
            .map(|(_, body)| body.clone())
    };

    let response = match body {
        Some(bytes) => {
            let mut head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .into_bytes();
            head.extend_from_slice(&bytes);
            head
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

// ------------------------------------------------------------------ fixture

/// 建一个独占的临时目录。不引 `tempfile`：一个 dev-dependency 换两行代码不值得。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-install-{tag}-{}-{:?}",
        std::process::id(),
        thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("能建临时目录");
    dir
}

/// 造一份形如发布产物的 `yunjian-<version>-<target>.tar.gz`，返回归档字节与其 SHA-256。
///
/// 归档里那个 `yunjian` 是一段可执行的 shell，不是真二进制：本测试要证明的是脚本把**正确
/// 的字节**放到了正确的位置，编译一个真 CLI 只会让门禁慢十分钟而断言力度不变。
fn build_archive(marker: &str) -> (Vec<u8>, String) {
    let stage = scratch(&format!("stage-{marker}"));
    let binary = stage.join("yunjian");
    fs::write(&binary, format!("#!/bin/sh\necho {marker}\n")).expect("能写占位二进制");

    let archive = stage.join("asset.tar.gz");
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage)
        .arg("yunjian")
        .status()
        .expect("能起 tar");
    assert!(status.success(), "tar 打包失败");

    let bytes = fs::read(&archive).expect("能读回归档");
    let digest = sha256_hex(&bytes);
    let _ = fs::remove_dir_all(&stage);
    (bytes, digest)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `sha256sum` 的输出格式：`<hex>  <filename>`。脚本只取第一段，但格式必须真实。
fn sha256sum_line(digest: &str, filename: &str) -> Vec<u8> {
    format!("{digest}  {filename}\n").into_bytes()
}

struct Installed {
    status: std::process::ExitStatus,
    stderr: String,
    home: PathBuf,
}

impl Installed {
    /// 默认安装目录：脚本里写的是 `$HOME/.local/bin`，所以测试**覆盖 HOME 而不是
    /// `YUNJIAN_INSTALL_DIR`**——后者会把「默认值对不对」这条断言绕过去。
    fn default_binary(&self) -> PathBuf {
        self.home.join(".local/bin/yunjian")
    }
}

fn run_install(server: &MockRelease, tag: &str, version: &str) -> Installed {
    let home = scratch(&format!("home-{tag}"));
    let script = repo_root().join("scripts/install.sh");
    let output = Command::new("sh")
        .arg(&script)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("YUNJIAN_VERSION", version)
        .env("YUNJIAN_BASE_URL", server.base_url())
        .output()
        .expect("能起 install.sh");

    Installed {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        home,
    }
}

// ------------------------------------------------------------------ 安装脚本

// 下面凡是真的去 `sh scripts/install.sh` 的用例，都挂 `cfg_attr(not(unix), ignore)`。
// `install.sh` 是 POSIX 脚本，Windows runner 上没有语义等价的 `sh`——不门控就会在跨平台
// 矩阵里必红，而红的原因与被测契约无关。选 `ignore` 而不是 `#[cfg(unix)]`：前者在测试输出
// 里留下一行 `ignored` 及理由，后者会让用例在 Windows 上凭空消失，看起来像「全都通过了」。
// Windows 侧的安装路径由 `install.ps1` 单独负责，不拿它来这几条里凑数。

#[test]
#[cfg_attr(
    not(unix),
    ignore = "install.sh 是 POSIX 脚本，非 unix 平台无可用 sh；Windows 侧由 install.ps1 负责"
)]
fn a_verified_asset_lands_in_the_default_directory() {
    let (archive, digest) = build_archive("happy");
    let routes = HashMap::from([
        (".tar.gz".to_owned(), archive.clone()),
        (
            ".sha256".to_owned(),
            sha256sum_line(&digest, "yunjian.tar.gz"),
        ),
    ]);
    let server = MockRelease::start(routes, Vec::new());

    let run = run_install(&server, "happy", "v9.9.9");
    assert!(
        run.status.success(),
        "安装应当成功：exit={:?}\n{}",
        run.status.code(),
        run.stderr
    );

    let installed = run.default_binary();
    assert!(
        installed.is_file(),
        "二进制应当落在默认目录 $HOME/.local/bin：{}\n{}",
        installed.display(),
        run.stderr
    );
    assert!(
        fs::read_to_string(&installed)
            .expect("能读回已安装文件")
            .contains("happy"),
        "落盘的必须是归档里那份字节"
    );

    // 校验和被**真的算过**：脚本只在比对通过后才打这一行，且行里带实算出的摘要。
    assert!(
        run.stderr.contains("校验和通过"),
        "必须报告校验和已核对：\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains(&digest),
        "报告里的摘要必须是归档的真实 SHA-256 {digest}：\n{}",
        run.stderr
    );

    // 下一步必须点名取语料：装完但没有语料的 CLI 只会退出 3。
    assert!(
        run.stderr.contains("yunjian corpus fetch"),
        "必须给出下一步：\n{}",
        run.stderr
    );

    let _ = fs::remove_dir_all(&run.home);
}

#[test]
#[cfg_attr(
    not(unix),
    ignore = "install.sh 是 POSIX 脚本，非 unix 平台无可用 sh；Windows 侧由 install.ps1 负责"
)]
fn a_tampered_asset_aborts_with_exit_three_and_installs_nothing() {
    let (archive, digest) = build_archive("tampered");
    // 摘要照发原件的，归档尾部改一个字节：这正是「传输被换掉」的形状。
    let mut corrupted = archive;
    corrupted.push(0x00);

    let routes = HashMap::from([
        (".tar.gz".to_owned(), corrupted),
        (
            ".sha256".to_owned(),
            sha256sum_line(&digest, "yunjian.tar.gz"),
        ),
    ]);
    let server = MockRelease::start(routes, Vec::new());

    let run = run_install(&server, "tampered", "v9.9.9");
    assert_eq!(
        run.status.code(),
        Some(3),
        "校验和不匹配是「拿到的文件不对」，应当退出 3：\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("校验和不匹配"),
        "必须说清失败原因：\n{}",
        run.stderr
    );
    assert!(
        !run.default_binary().exists(),
        "校验失败必须一个文件都不装：{}",
        run.default_binary().display()
    );

    let _ = fs::remove_dir_all(&run.home);
}

/// Linux 上 musl 是首选、gnu 是回退。这条断言盯的是回退链本身还在。
#[cfg(target_os = "linux")]
#[test]
fn a_missing_musl_asset_falls_back_to_the_gnu_build() {
    let (archive, digest) = build_archive("fallback");
    let routes = HashMap::from([
        (".tar.gz".to_owned(), archive),
        (
            ".sha256".to_owned(),
            sha256sum_line(&digest, "yunjian.tar.gz"),
        ),
    ]);
    let server = MockRelease::start(routes, vec!["musl".to_owned()]);

    let run = run_install(&server, "fallback", "v9.9.9");
    assert!(
        run.status.success(),
        "musl 缺失应当回退到 gnu 而不是失败：exit={:?}\n{}",
        run.status.code(),
        run.stderr
    );
    assert!(run.default_binary().is_file(), "回退路径也要真的装上");
    assert!(
        run.stderr.contains("unknown-linux-gnu"),
        "应当报告实际用了哪个目标：\n{}",
        run.stderr
    );

    let _ = fs::remove_dir_all(&run.home);
}

#[test]
#[cfg_attr(
    not(unix),
    ignore = "install.sh 是 POSIX 脚本，非 unix 平台无可用 sh；Windows 侧由 install.ps1 负责"
)]
fn an_unset_version_is_not_required_to_be_a_v_prefixed_tag() {
    let (archive, digest) = build_archive("bare-version");
    let routes = HashMap::from([
        (".tar.gz".to_owned(), archive),
        (
            ".sha256".to_owned(),
            sha256sum_line(&digest, "yunjian.tar.gz"),
        ),
    ]);
    let server = MockRelease::start(routes, Vec::new());

    // `0.1.0` 与 `v0.1.0` 都要收：用户抄的是版本号，不一定带 `v`。
    let run = run_install(&server, "bare-version", "9.9.9");
    assert!(
        run.status.success(),
        "不带 v 的版本号也应当接受：\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("v9.9.9"),
        "应当规范化成 tag 形态：\n{}",
        run.stderr
    );

    let _ = fs::remove_dir_all(&run.home);
}

// ------------------------------------------------------------------ 两个脚本同源

/// 两个脚本必须认同一组环境变量名。名字分叉的现象是「文档照 sh 写，Windows 用户照做无效」。
#[test]
fn both_installers_honour_the_same_environment_variable_names() {
    let root = repo_root();
    let sh = fs::read_to_string(root.join("scripts/install.sh")).expect("install.sh 可读");
    let ps1 = fs::read_to_string(root.join("scripts/install.ps1")).expect("install.ps1 可读");

    for name in [
        "YUNJIAN_VERSION",
        "YUNJIAN_INSTALL_DIR",
        "YUNJIAN_BASE_URL",
        "YUNJIAN_API_URL",
    ] {
        assert!(sh.contains(name), "install.sh 缺少 {name}");
        assert!(ps1.contains(name), "install.ps1 缺少 {name}");
    }

    // 默认安装目录也不许分叉。
    assert!(sh.contains(".local/bin"), "install.sh 的默认目录变了");
    assert!(ps1.contains(".local\\bin"), "install.ps1 的默认目录变了");
}

// ------------------------------------------------------------------ README 契约

/// 取出 README 里全部 ```json 代码块的内容。
fn json_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        match current.as_mut() {
            Some(buf) => {
                if line.trim_start().starts_with("```") {
                    blocks.push(std::mem::take(buf));
                    current = None;
                } else {
                    buf.push_str(line);
                    buf.push('\n');
                }
            }
            None => {
                if line.trim_start() == "```json" {
                    current = Some(String::new());
                }
            }
        }
    }
    blocks
}

/// 两种客户端配置形态**都**要在 README 里，且各自能被解析。
///
/// 只给一种就等于把另一半用户送进「配了但连不上」：两边顶层键不同，`command` 的类型也
/// 不同，把一种套到另一种上不会报错，只是永远握不上手。
#[test]
fn the_readme_carries_both_mcp_client_shapes_and_each_one_parses() {
    let readme = read_root_readme();
    let blocks = json_blocks(&readme);
    assert!(!blocks.is_empty(), "README 里应当有 json 代码块");

    let mut claude = None;
    let mut opencode = None;
    for block in &blocks {
        let value: serde_json::Value = match serde_json::from_str(block) {
            Ok(value) => value,
            // README 里的每个 json 块都必须是合法 JSON——一个粘不进配置文件的样例比没有
            // 样例更坏：用户会以为自己配错了。
            Err(error) => panic!("README 的 json 块解析失败：{error}\n{block}"),
        };
        if value.get("mcpServers").is_some() {
            claude = Some(value);
        } else if value.get("mcp").is_some() {
            opencode = Some(value);
        }
    }

    let claude = claude.expect("README 必须给出 Claude Desktop 的 `mcpServers` 形态");
    let opencode = opencode.expect("README 必须给出 OpenCode 的 `mcp` 形态");

    // Claude：`command` 是字符串，参数另放 `args`。
    let entry = &claude["mcpServers"]["yunjian"];
    assert_eq!(
        entry["command"], "yunjian",
        "Claude 的 command 必须是字符串：{entry}"
    );
    assert!(
        entry["command"].is_string(),
        "Claude 的 command 是字符串而不是数组：{entry}"
    );
    assert_eq!(entry["args"], serde_json::json!(["mcp"]), "{entry}");

    // OpenCode：`command` 是含参数的数组，另有 `type` 与 `enabled`。
    let entry = &opencode["mcp"]["yunjian"];
    assert!(
        entry["command"].is_array(),
        "OpenCode 的 command 必须是数组：{entry}"
    );
    assert_eq!(
        entry["command"],
        serde_json::json!(["yunjian", "mcp"]),
        "{entry}"
    );
    assert_eq!(entry["type"], "local", "{entry}");
    assert_eq!(entry["enabled"], true, "{entry}");

    // 两种形态在结构上确实不同——这正是必须两个都给的理由。
    assert!(
        claude.get("mcp").is_none() && opencode.get("mcpServers").is_none(),
        "两份样例不该互相污染顶层键"
    );
}

/// 快速开始必须是「装 → 取语料 → 看到结果」，且顺序不能乱。
///
/// 顺序是产品语义：先 `search` 后 `fetch` 的读者会先撞上一次退出 3。
#[test]
fn the_quickstart_shows_install_then_fetch_then_search_in_order() {
    let readme = read_root_readme();
    let start = readme
        .find("## 快速开始")
        .expect("README 必须有快速开始一节");
    let rest = &readme[start..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |at| at + 3);
    let quickstart = &rest[..end];

    let install = quickstart
        .find("scripts/install.sh")
        .expect("快速开始第一步必须是安装脚本");
    let fetch = quickstart
        .find("yunjian corpus fetch")
        .expect("快速开始第二步必须是取语料");
    let search = quickstart
        .find("yunjian search 明月")
        .expect("快速开始第三步必须是 `yunjian search 明月`");

    assert!(
        install < fetch && fetch < search,
        "顺序必须是 安装 → corpus fetch → search 明月，实测 {install}/{fetch}/{search}"
    );

    // Windows 用户也要有一条路，否则「快速开始」只对一半平台成立。
    assert!(
        quickstart.contains("scripts/install.ps1"),
        "快速开始必须同时给出 Windows 的安装方式"
    );
}

/// `## 与 LLM 协作` 必须在 `<details>` 里，并写清两条流与四个退出码。
#[test]
fn the_llm_section_is_collapsible_and_states_the_stream_and_exit_code_contract() {
    let readme = read_root_readme();
    let start = readme
        .find("## 与 LLM 协作")
        .expect("README 必须有「与 LLM 协作」一节");
    let section = &readme[start..];
    let end = section[3..]
        .find("\n## ")
        .map_or(section.len(), |at| at + 3);
    let section = &section[..end];

    assert!(
        section.contains("<details>") && section.contains("</details>"),
        "该段必须收在 <details> 里，否则 README 会被它撑爆"
    );
    assert!(
        section.contains("stdout") && section.contains("stderr"),
        "必须写清两条流的分工"
    );

    // 四个退出码逐个点名。少写一个就会有调用方把它当成未定义行为。
    for code in ["| 0 ", "| 1 ", "| 2 ", "| 3 "] {
        assert!(section.contains(code), "退出码表缺少 {code}");
    }
    // 1 与 3 的区别是这份契约里最贵的一条，必须显式写出来。
    assert!(
        section.contains("1 和 3 不能混"),
        "必须点明「结果为空」与「数据不可用」不是一回事"
    );
    // 信封的五个顶层字段。
    for field in [
        "schema_version",
        "\"command\"",
        "\"status\"",
        "warnings",
        "\"data\"",
    ] {
        assert!(section.contains(field), "信封说明缺少字段 {field}");
    }
}
