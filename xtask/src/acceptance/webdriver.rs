//! W3C WebDriver 客户端，用来驱动 `tauri-driver` 背后的真实 WebView。
//!
//! # 为什么手写一个客户端而不是引一个库
//!
//! 需要的是 W3C 协议的一个很小的子集：建 session、按 CSS 选择器找元素、读文本、
//! 点击、输入、执行脚本、销毁 session。`ureq` 已经在 xtask 的依赖树里（`verify-sources`
//! 抓上游 LICENSE 用它），而现成的 WebDriver 库要么拖一整棵 async 运行时，要么把
//! 「会话建不起来」这件事包装成一个看不出原因的错误——而在本 harness 里，**会话建不
//! 起来正是需要如实报告的那个观测结果**，不能被封装掉。
//!
//! # 这里刻意没有 mock
//!
//! 一个「WebDriver 不可用时改用假响应」的实现会把真机验收退化成单元测试，报告会变绿
//! 而没有任何一行像素被验证过。所以本模块只有真实 HTTP，握手失败就是失败，由调用方
//! 记成 `NOT EXECUTED` 并写明原因。

use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::Background;

/// `tauri-driver` 的中介端口。
const DRIVER_PORT: u16 = 4444;
/// 底层原生 WebDriver 端口。显式指定，避免与宿主机上别的东西撞。
const NATIVE_PORT: u16 = 4457;
/// 建 session 的等待上限。本机实测：握手不成功时它不会报错，只是永不返回，
/// 所以超时是这条路径上唯一的判定手段。
const SESSION_TIMEOUT: Duration = Duration::from_secs(45);

/// 一个活着的 WebDriver 会话。
pub(crate) struct Session {
    base: String,
    id: String,
    agent: ureq::Agent,
    /// 持有 driver 进程，Drop 时拆除。
    _driver: Background,
}

/// 握手失败时的观测结果。**它本身就是证据**，不是一个要被吞掉的错误。
pub(crate) struct HandshakeFailure {
    pub(crate) detail: String,
}

/// 尝试建立会话。
///
/// `app` 是被测产物的绝对路径，`env` 是要透传给应用进程的环境变量。
///
/// # Errors
///
/// 不返回错误：握手失败以 [`HandshakeFailure`] 的形态返回，因为调用方要把它写进报告。
/// 只有 harness 自身无法启动 driver 时才是 `Err`。
pub(crate) fn connect(
    app: &std::path::Path,
    env: &[(&str, String)],
) -> Result<Result<Session, HandshakeFailure>> {
    if which("tauri-driver").is_none() {
        return Ok(Err(HandshakeFailure {
            detail: "`tauri-driver` 不在 PATH 上。".to_owned(),
        }));
    }
    if which("WebKitWebDriver").is_none() {
        return Ok(Err(HandshakeFailure {
            detail: "`WebKitWebDriver` 不在 PATH 上（Linux 上它是 tauri-driver 的底层驱动）。"
                .to_owned(),
        }));
    }

    let mut command = Command::new("tauri-driver");
    command
        .arg("--port")
        .arg(DRIVER_PORT.to_string())
        .arg("--native-port")
        .arg(NATIVE_PORT.to_string());
    for (key, value) in env {
        command.env(key, value);
    }
    // WebKitGTK 的自动化会话要通过 session D-Bus 通告自己。容器里 DBUS_SESSION_BUS_ADDRESS
    // 常常指着一个已经死掉的 socket，于是应用侧连不上总线。`dbus-run-session` 起一条
    // 新的并把它交给整棵子进程树。
    let mut command = if which("dbus-run-session").is_some() {
        let mut wrapper = Command::new("dbus-run-session");
        wrapper.arg("--");
        wrapper.arg("tauri-driver");
        wrapper
            .arg("--port")
            .arg(DRIVER_PORT.to_string())
            .arg("--native-port")
            .arg(NATIVE_PORT.to_string());
        for (key, value) in env {
            wrapper.env(key, value);
        }
        wrapper
    } else {
        command
    };

    let driver = Background::spawn("tauri-driver", &mut command)?;
    let base = format!("http://127.0.0.1:{DRIVER_PORT}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(SESSION_TIMEOUT))
        .build()
        .into();

    // driver 起来之前 POST 会直接连不上，先等端口。
    let ready = Background::wait_until(Duration::from_secs(10), || {
        std::net::TcpStream::connect(("127.0.0.1", DRIVER_PORT)).is_ok()
    });
    if !ready {
        return Ok(Err(HandshakeFailure {
            detail: format!("`tauri-driver` 起来后 10 秒内没有监听 {DRIVER_PORT} 端口。"),
        }));
    }

    let body = json!({
        "capabilities": {
            "alwaysMatch": {
                "browserName": "wry",
                "tauri:options": { "application": app.to_string_lossy() }
            }
        }
    });
    let response = post_json(&agent, &format!("{base}/session"), &body);
    let value: Value = match response {
        Ok(value) => value,
        Err(error) => {
            return Ok(Err(HandshakeFailure {
                detail: format!(
                    "`POST /session` 未能返回会话：{error}。\
                     本机实测：应用进程真的起来了、真实窗口映射成功、\
                     WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` \
                     从不向它建立连接，请求一直挂到超时。"
                ),
            }));
        }
    };
    let Some(id) = value
        .get("value")
        .and_then(|v| v.get("sessionId"))
        .and_then(Value::as_str)
    else {
        return Ok(Err(HandshakeFailure {
            detail: format!("建会话响应里没有 sessionId：{value}"),
        }));
    };

    Ok(Ok(Session {
        base,
        id: id.to_owned(),
        agent,
        _driver: driver,
    }))
}

impl Session {
    /// 按 CSS 选择器取元素 id。
    ///
    /// # Errors
    ///
    /// 元素不存在或协议响应形状不对。
    pub(crate) fn find(&self, css: &str) -> Result<String> {
        let value = post_json(
            &self.agent,
            &format!("{}/session/{}/element", self.base, self.id),
            &json!({ "using": "css selector", "value": css }),
        )
        .with_context(|| format!("按 `{css}` 找元素失败"))?;
        let Some(object) = value.get("value").and_then(Value::as_object) else {
            bail!("按 `{css}` 找元素的响应形状不对：{value}");
        };
        object
            .values()
            .find_map(Value::as_str)
            .map(str::to_owned)
            .with_context(|| format!("按 `{css}` 找到的元素没有引用 id：{value}"))
    }

    /// 读元素文本。
    ///
    /// # Errors
    ///
    /// 元素不存在或请求失败。
    pub(crate) fn text(&self, css: &str) -> Result<String> {
        let element = self.find(css)?;
        let mut response = self
            .agent
            .get(format!(
                "{}/session/{}/element/{element}/text",
                self.base, self.id
            ))
            .call()
            .with_context(|| format!("读 `{css}` 的文本失败"))?;
        let raw = response
            .body_mut()
            .read_to_string()
            .context("读取读文本响应体失败")?;
        let value: Value = serde_json::from_str(&raw).context("解析读文本响应失败")?;
        Ok(value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// 往元素里输入文本。
    ///
    /// # Errors
    ///
    /// 元素不存在或请求失败。
    pub(crate) fn send_keys(&self, css: &str, text: &str) -> Result<()> {
        let element = self.find(css)?;
        post_json(
            &self.agent,
            &format!("{}/session/{}/element/{element}/value", self.base, self.id),
            &json!({ "text": text }),
        )
        .with_context(|| format!("往 `{css}` 输入失败"))?;
        Ok(())
    }

    /// 点击元素。
    ///
    /// # Errors
    ///
    /// 元素不存在或请求失败。
    pub(crate) fn click(&self, css: &str) -> Result<()> {
        let element = self.find(css)?;
        post_json(
            &self.agent,
            &format!("{}/session/{}/element/{element}/click", self.base, self.id),
            &json!({}),
        )
        .with_context(|| format!("点击 `{css}` 失败"))?;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // 尽力销毁会话；失败也不能 panic，否则会盖掉真正的失败原因。
        let _ = self
            .agent
            .delete(format!("{}/session/{}", self.base, self.id))
            .call();
    }
}

/// 发一个 JSON POST 并把响应体解析成 [`Value`]。
///
/// 刻意自己序列化而不是开 `ureq` 的 `json` 特性：`ureq` 是工作区共享依赖，为一个开发
/// 工具的子命令给它加特性会影响到 `yunjian-core` 与 `yunjian-voice` 的构建。
///
/// # Errors
///
/// 请求失败、响应体读不出，或响应体不是 JSON。
fn post_json(agent: &ureq::Agent, url: &str, body: &Value) -> Result<Value> {
    let encoded = serde_json::to_string(body).context("序列化请求体失败")?;
    let mut response = agent
        .post(url)
        .content_type("application/json")
        .send(&encoded)
        .with_context(|| format!("POST {url} 失败"))?;
    let raw = response
        .body_mut()
        .read_to_string()
        .with_context(|| format!("读取 {url} 响应体失败"))?;
    serde_json::from_str(&raw).with_context(|| format!("{url} 的响应体不是 JSON：{raw}"))
}

/// PATH 查找。刻意不依赖 `which` crate：一个目录遍历不值得多一条依赖。
pub(crate) fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}
