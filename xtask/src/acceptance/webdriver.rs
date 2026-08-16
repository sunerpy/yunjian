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

/// 建 session 的等待上限。本机实测：握手不成功时它不会报错，只是永不返回，
/// 所以超时是这条路径上唯一的判定手段。
const SESSION_TIMEOUT: Duration = Duration::from_secs(45);
/// 等 driver 开始监听自己那个端口的上限。
///
/// 30 秒不是随手加的余量：本机实测第二、第三条会话的 driver 偶尔在 10 秒内起不来
/// （上一条会话正在拆除，机器同时在跑别的东西），而那会写成一条「driver 没有监听」的
/// 未执行——一个纯粹的时序结论占掉了一条断言。
const PORT_LISTEN_TIMEOUT: Duration = Duration::from_secs(30);

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

    // 端口每次现取，不写死。写死过 4444/4457，代价是一次高成本误诊：上一轮泄漏的
    // `tauri-driver` 还占着 4444，本轮自己那个绑不上端口直接退出，而 `POST /session`
    // 打到了**那个旧 driver** 上并挂到超时——判词于是写成「`WebKitWebDriver` 从不建立
    // 连接」，读起来像 WebKit 的问题，真因只是一个没退干净的进程。
    // 让内核给一个空闲端口，这类跨轮次污染就不存在了。
    let (driver_port, native_port) = free_port_pair()?;

    let mut command = Command::new("tauri-driver");
    command
        .arg("--port")
        .arg(driver_port.to_string())
        .arg("--native-port")
        .arg(native_port.to_string());
    for (key, value) in env {
        command.env(key, value);
    }

    let driver = Background::spawn("tauri-driver", &mut command)?;
    let base = format!("http://127.0.0.1:{driver_port}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(SESSION_TIMEOUT))
        .build()
        .into();

    // driver 起来之前 POST 会直接连不上，先等端口。
    let ready = Background::wait_until(PORT_LISTEN_TIMEOUT, || {
        std::net::TcpStream::connect(("127.0.0.1", driver_port)).is_ok()
    });
    if !ready {
        return Ok(Err(HandshakeFailure {
            detail: format!(
                "`tauri-driver` 起来后 {} 秒内没有监听 {driver_port} 端口。",
                PORT_LISTEN_TIMEOUT.as_secs()
            ),
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

    /// 按 CSS 选择器取**全部**匹配元素的引用 id，顺序即文档顺序。
    ///
    /// 检索结果那种「同一个选择器命中多行、要按行内文字挑一行」的驱动步骤必须用它：
    /// [`Session::find`] 只给第一个，用它去点第 n 行会静默点到第一行。
    ///
    /// # Errors
    ///
    /// 请求失败或响应形状不对。一个都没命中**不是错误**，返回空 `Vec`。
    pub(crate) fn find_all(&self, css: &str) -> Result<Vec<String>> {
        let value = post_json(
            &self.agent,
            &format!("{}/session/{}/elements", self.base, self.id),
            &json!({ "using": "css selector", "value": css }),
        )
        .with_context(|| format!("按 `{css}` 找全部元素失败"))?;
        let Some(items) = value.get("value").and_then(Value::as_array) else {
            bail!("按 `{css}` 找全部元素的响应形状不对：{value}");
        };
        Ok(items
            .iter()
            .filter_map(|item| {
                item.as_object()
                    .and_then(|object| object.values().find_map(Value::as_str))
                    .map(str::to_owned)
            })
            .collect())
    }

    /// 读元素文本。
    ///
    /// # Errors
    ///
    /// 元素不存在或请求失败。
    pub(crate) fn text(&self, css: &str) -> Result<String> {
        let element = self.find(css)?;
        self.element_text(&element)
    }

    /// # Errors
    ///
    /// 元素引用已失效或请求失败。
    pub(crate) fn element_text(&self, element: &str) -> Result<String> {
        let value = self.get_json(&format!(
            "{}/session/{}/element/{element}/text",
            self.base, self.id
        ))?;
        Ok(value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// 读元素的 **IDL 属性**（property），不是 HTML attribute。
    ///
    /// 检索框里现在是哪几个字只能这样读：`<input>` 的 `textContent` 恒为空，
    /// 因此 [`Session::text`] 对它永远返回空串——那会把「字符没落进框里」这个失败
    /// 和「读法不对」这个 harness 缺陷混成同一个形态。
    ///
    /// # Errors
    ///
    /// 元素不存在或请求失败。属性不存在时协议返回 `null`，这里映射成空串。
    pub(crate) fn property(&self, css: &str, name: &str) -> Result<String> {
        let element = self.find(css)?;
        let value = self.get_json(&format!(
            "{}/session/{}/element/{element}/property/{name}",
            self.base, self.id
        ))?;
        Ok(value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// # Errors
    ///
    /// 元素引用已失效、元素不可交互，或请求失败。
    pub(crate) fn click_element(&self, element: &str) -> Result<()> {
        post_json(
            &self.agent,
            &format!("{}/session/{}/element/{element}/click", self.base, self.id),
            &json!({}),
        )
        .context("点击元素引用失败")?;
        Ok(())
    }

    /// 截当前 WebView 的图并写到 `path`。
    ///
    /// 与 OS 通道那条 X `GetImage` 截图**不是**一回事：这里问的是 WebView 自己渲染成了
    /// 什么，因此不受「内容合成到了一块 X 读不回的表面」影响。报告里引用一张截图之前
    /// 必须真的把它写出来——引用一个不存在的文件会让读报告的人以为看过了证据。
    ///
    /// # Errors
    ///
    /// 请求失败、响应不是 base64，或写文件失败。
    pub(crate) fn screenshot(&self, path: &std::path::Path) -> Result<()> {
        let value = self.get_json(&format!("{}/session/{}/screenshot", self.base, self.id))?;
        let encoded = value
            .get("value")
            .and_then(Value::as_str)
            .context("截图响应里没有 base64 数据")?;
        let bytes = decode_base64(encoded).context("截图响应不是合法 base64")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建 {} 失败", parent.display()))?;
        }
        std::fs::write(path, bytes).with_context(|| format!("写 {} 失败", path.display()))
    }

    fn get_json(&self, url: &str) -> Result<Value> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .with_context(|| format!("GET {url} 失败"))?;
        let raw = response
            .body_mut()
            .read_to_string()
            .with_context(|| format!("读取 {url} 响应体失败"))?;
        serde_json::from_str(&raw).with_context(|| format!("{url} 的响应体不是 JSON：{raw}"))
    }

    /// 等一个元素出现，返回它是否在 `timeout` 内出现过。
    ///
    /// 建会话成功只说明 WebView 起来了，不说明 React 已经挂载。实测：会话返回后
    /// 立刻查 `[data-testid='search-input']` 得到 `no such element`，约两秒后才有。
    /// 没有这一步时那个时序会表现成一条 FAIL，读起来像「检索框不存在」——
    /// 而那是一个假故障，且会让人去改一个没坏的前端。
    ///
    /// 这里**只等元素出现，不放宽任何断言**：等到超时仍没有，调用方照样失败。
    pub(crate) fn wait_for(&self, css: &str, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.find(css).is_ok() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// 清空一个输入框。
    ///
    /// 走 W3C 的 `element/{id}/clear` 而**不是**发一串退格：受控 React 组件的 `value`
    /// 由状态决定，退格键改不动它，而 `clear` 会派发 `input` 事件让状态跟着变。
    ///
    /// # Errors
    ///
    /// 元素不存在或请求失败。
    pub(crate) fn clear(&self, css: &str) -> Result<()> {
        let element = self.find(css)?;
        post_json(
            &self.agent,
            &format!("{}/session/{}/element/{element}/clear", self.base, self.id),
            &json!({}),
        )
        .with_context(|| format!("清空 `{css}` 失败"))?;
        Ok(())
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

/// 向内核要两个此刻空闲的本地端口。
///
/// 先 bind 到 0 号端口拿到内核分配的号，再释放它交给 driver。这中间有一个很小的竞争窗口，
/// 而代价是可承受的：撞上了表现为 driver 起不来，那是一条如实的握手失败，
/// 不会伪装成别的结论。
fn free_port_pair() -> Result<(u16, u16)> {
    let mut ports = Vec::with_capacity(2);
    let mut listeners = Vec::with_capacity(2);
    for _ in 0..2 {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").context("向内核申请空闲端口失败")?;
        ports.push(listener.local_addr().context("读取已分配端口失败")?.port());
        listeners.push(listener);
    }
    drop(listeners);
    Ok((ports[0], ports[1]))
}

/// PATH 查找。刻意不依赖 `which` crate：一个目录遍历不值得多一条依赖。
pub(crate) fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

/// 解一段标准 base64。
///
/// 刻意不引 `base64` crate：整个工作区只有截图这一处需要解码，而 W3C 截图响应是固定
/// 的标准字母表加 `=` 填充，一个查表循环足够，且**不能**为它给共享依赖树再加一个包。
fn decode_base64(encoded: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [u8::MAX; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = u8::try_from(index).expect("base64 字母表只有 64 个位置");
    }

    let mut out = Vec::with_capacity(encoded.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in encoded.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = lookup[byte as usize];
        if value == u8::MAX {
            bail!("base64 里出现了字母表外的字节 0x{byte:02x}");
        }
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            let shifted = (accumulator >> bits) & 0xFF;
            out.push(u8::try_from(shifted).expect("掩码到 8 位后必然可转"));
        }
    }
    Ok(out)
}
