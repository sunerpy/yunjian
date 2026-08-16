//! 托盘宿主：一个最小的 `org.kde.StatusNotifierWatcher`，让托盘断言可执行。
//!
//! # 为什么这不是「模拟托盘」
//!
//! Xvfb + Openbox 里没有任何托盘宿主，此前 `tray_icon_correct` 与 `app_exits_cleanly`
//! 都因此记未执行。但**缺的不是显示硬件，是总线上的一个名字**：Linux 上托盘走
//! StatusNotifierItem/AppIndicator，图标与菜单都在 D-Bus 上，像素只是宿主最后画出来的
//! 结果。`libayatana-appindicator` 起来时先看 `org.kde.StatusNotifierWatcher` 在不在，
//! 在就把自己注册上去并把图标路径、菜单布局全部通过总线公开。
//!
//! 所以本模块**不伪造托盘项**：它只提供那个宿主名字，随后从协议侧读应用**真的**发布了
//! 什么。图标是应用自己写到 `IconThemePath` 下的那个 PNG 文件，菜单是应用自己
//! `MenuBuilder` 建的那四项，「退出」是通过 `com.canonical.dbusmenu.Event` 投给应用
//! 自己的处理器——与真人在 GNOME 上点它走的是同一条路。
//!
//! # 对照实验
//!
//! 本机实测（同一台机、同一个产物、同一块 Xvfb）：
//!
//! - 声明 `org.kde.StatusNotifierWatcher`：应用 3 秒内发起
//!   `RegisterStatusNotifierItem`，随后 `IconName` / `Menu` / `GetLayout` 全部可读，
//!   点「退出」应用以退出码 0 结束。
//! - **不声明**那个名字（其余完全相同）：90 秒内一次注册都没有。
//!
//! 后者正是此前那两条断言未执行时的处境。因此「没有托盘宿主」是一个可以由软件补上的
//! 缺口，不是环境限制——这条判据本身值得记住：**先问能不能软件模拟，再判环境阻塞。**
//!
//! # 为什么自己起 `dbus-daemon` 而不用 `dbus-run-session`
//!
//! 观测方（本进程的 watcher）与被观测方（应用子进程）必须在**同一条** session 总线上。
//! `dbus-run-session` 只能把新总线交给它自己拉起的那棵进程树，而 harness 是先起 watcher
//! 再 spawn 应用；把地址写进本进程环境要 `std::env::set_var`，它在 2024 版次是 `unsafe`，
//! 本工作区禁用。于是自己起 daemon 拿到地址，watcher 显式连它，应用那侧用
//! `Command::env` 传同一个地址——两边都不碰进程环境。

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use zbus::zvariant::{Array, Dict, OwnedValue, Structure, Value};

use super::Background;

/// 托盘项注册的等待上限。本机实测注册在 3 秒内发生。
const REGISTER_TIMEOUT: Duration = Duration::from_secs(45);
/// 点「退出」之后等进程结束的上限。
const EXIT_TIMEOUT: Duration = Duration::from_secs(20);

/// 应用在总线上发布的那个托盘项。
pub(crate) struct TrayItem {
    /// 发布方的唯一名（`:1.4` 之类）。
    bus: String,
    /// 托盘项对象路径。
    path: String,
}

/// 菜单里的一项。
pub(crate) struct MenuEntry {
    pub(crate) id: i32,
    pub(crate) label: String,
}

/// 一条只在本次验收里存在的 session 总线。
///
/// 它是**被托管的直接子进程**，因此拆除是确定的。以前托盘与 driver 都走
/// `dbus-run-session -- <程序>`，那让真正占端口的 `tauri-driver` 变成孙进程：
/// 杀掉被托管的 `dbus-run-session` 只会把它留成孤儿，于是下一条会话报「端口已被占」。
pub(crate) struct SessionBus {
    address: String,
    _daemon: Background,
}

/// 活着的托盘宿主：在一条已有的总线上声明 watcher 名。
pub(crate) struct TrayHost {
    registered: Arc<Mutex<Vec<TrayItem>>>,
    connection: zbus::blocking::Connection,
}

struct Watcher {
    registered: Arc<Mutex<Vec<TrayItem>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        // `libayatana-appindicator` 传的是**对象路径**而不是总线名（规范允许两种），
        // 此时项在「消息发送方的唯一名 + 那个路径」上。按规范那样当成总线名去连会得到
        // 一个 `InvalidName`，读起来像协议不兼容，其实只是取错了字段。
        let sender = header.sender().map(ToString::to_string).unwrap_or_default();
        let item = if service.starts_with('/') {
            TrayItem {
                bus: sender,
                path: service.to_owned(),
            }
        } else {
            TrayItem {
                bus: service.to_owned(),
                path: "/StatusNotifierItem".to_owned(),
            }
        };
        if let Ok(mut items) = self.registered.lock() {
            items.push(item);
        }
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registered
            .lock()
            .map(|items| items.iter().map(|item| item.path.clone()).collect())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

impl SessionBus {
    /// # Errors
    ///
    /// `dbus-daemon` 不在 PATH 上，或它没有打印出总线地址。
    pub(crate) fn start() -> Result<Self> {
        let daemon = super::webdriver::which("dbus-daemon")
            .context("`dbus-daemon` 不在 PATH 上，无法为托盘断言提供 session 总线")?;
        let mut command = Command::new(daemon);
        command
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().context("启动 dbus-daemon 失败")?;
        let stdout = child
            .stdout
            .take()
            .context("dbus-daemon 没有给出 stdout 管道")?;
        let mut address = String::new();
        BufReader::new(stdout)
            .read_line(&mut address)
            .context("读 dbus-daemon 打印的总线地址失败")?;
        let address = address.trim().to_owned();
        if address.is_empty() {
            bail!("dbus-daemon 没有打印出 session 总线地址");
        }
        Ok(Self {
            address,
            _daemon: Background::adopt("dbus-daemon", child),
        })
    }

    /// 要透传给子进程的环境变量，让它们连上这条总线。
    pub(crate) fn env(&self) -> (&'static str, String) {
        ("DBUS_SESSION_BUS_ADDRESS", self.address.clone())
    }
}

impl TrayHost {
    /// 在 `bus` 上声明 `org.kde.StatusNotifierWatcher`。
    ///
    /// # Errors
    ///
    /// 连不上总线，或那个名字已被占用。
    pub(crate) fn claim(bus: &SessionBus) -> Result<Self> {
        let registered = Arc::new(Mutex::new(Vec::new()));
        let connection = zbus::blocking::connection::Builder::address(bus.address.as_str())
            .context("按地址构造 D-Bus 连接失败")?
            .name("org.kde.StatusNotifierWatcher")
            .context("声明 org.kde.StatusNotifierWatcher 失败")?
            .serve_at(
                "/StatusNotifierWatcher",
                Watcher {
                    registered: Arc::clone(&registered),
                },
            )
            .context("挂载 watcher 对象失败")?
            .build()
            .context("建立 watcher 连接失败")?;
        Ok(Self {
            registered,
            connection,
        })
    }

    /// 等应用注册托盘项。
    pub(crate) fn wait_for_item(&self) -> Option<TrayItem> {
        let deadline = Instant::now() + REGISTER_TIMEOUT;
        loop {
            if let Ok(items) = self.registered.lock()
                && let Some(item) = items.first()
            {
                return Some(TrayItem {
                    bus: item.bus.clone(),
                    path: item.path.clone(),
                });
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// 读托盘项的一个 `org.kde.StatusNotifierItem` 属性。
    ///
    /// # Errors
    ///
    /// 属性不存在或调用失败。
    pub(crate) fn property(&self, item: &TrayItem, name: &str) -> Result<String> {
        let reply = self
            .connection
            .call_method(
                Some(item.bus.as_str()),
                item.path.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.kde.StatusNotifierItem", name),
            )
            .with_context(|| format!("读托盘属性 {name} 失败"))?;
        let value: OwnedValue = reply
            .body()
            .deserialize()
            .with_context(|| format!("解析托盘属性 {name} 失败"))?;
        text_of(&value).with_context(|| format!("托盘属性 {name} 不是字符串或对象路径"))
    }

    /// 读托盘菜单的扁平项列表（有 `label` 的那些）。
    ///
    /// # Errors
    ///
    /// 取不到菜单路径，或 `GetLayout` 调用失败。
    pub(crate) fn menu_entries(&self, item: &TrayItem) -> Result<Vec<MenuEntry>> {
        let menu = self.property(item, "Menu")?;
        // 真人点开托盘菜单时宿主会先发这个。返回 false 表示「布局没变，直接用缓存的」，
        // 不是失败，所以结果刻意不参与判定。
        let _ = self.connection.call_method(
            Some(item.bus.as_str()),
            menu.as_str(),
            Some("com.canonical.dbusmenu"),
            "AboutToShow",
            &(0i32,),
        );
        let reply = self
            .connection
            .call_method(
                Some(item.bus.as_str()),
                menu.as_str(),
                Some("com.canonical.dbusmenu"),
                "GetLayout",
                &(0i32, -1i32, Vec::<String>::new()),
            )
            .context("调用 com.canonical.dbusmenu.GetLayout 失败")?;
        let (_revision, root): (u32, MenuNode) = reply
            .body()
            .deserialize()
            .context("解析 GetLayout 响应失败")?;
        let mut entries = Vec::new();
        collect_entries(&root, &mut entries);
        Ok(entries)
    }

    /// 点菜单里的一项，走应用自己的 `on_menu_event`。
    ///
    /// # Errors
    ///
    /// 取不到菜单路径或投递事件失败。
    pub(crate) fn activate(&self, item: &TrayItem, id: i32) -> Result<()> {
        let menu = self.property(item, "Menu")?;
        self.connection
            .call_method(
                Some(item.bus.as_str()),
                menu.as_str(),
                Some("com.canonical.dbusmenu"),
                "Event",
                &(id, "clicked", Value::from(0i32), 0u32),
            )
            .context("投递 com.canonical.dbusmenu.Event 失败")?;
        Ok(())
    }
}

/// 等一个进程结束，返回它的退出码。
pub(crate) fn wait_for_exit(child: &mut std::process::Child) -> Result<Option<i32>> {
    let deadline = Instant::now() + EXIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().context("查询应用进程状态失败")? {
            return Ok(Some(status.code().unwrap_or(-1)));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `com.canonical.dbusmenu.GetLayout` 返回的那棵树，签名 `(u(ia{sv}av))`。
///
/// 子节点声明成 `OwnedValue` 而不是递归的 `MenuNode`：线上签名里它们是 `av`，
/// 每个元素外面还包着一层 `Variant`，直接递归声明会得到一个签名不匹配的解析错误。
#[derive(serde::Deserialize, zbus::zvariant::Type)]
struct MenuNode {
    id: i32,
    props: std::collections::HashMap<String, OwnedValue>,
    children: Vec<OwnedValue>,
}

/// dbusmenu 的 `a{sv}` 里每个值还多包一层 `Variant`，直接取 `&str` 会失败。
fn text_of(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.to_string()),
        Value::ObjectPath(path) => Some(path.to_string()),
        Value::Value(inner) => text_of(inner),
        _ => None,
    }
}

fn collect_entries(node: &MenuNode, out: &mut Vec<MenuEntry>) {
    if let Some(label) = node.props.get("label").and_then(|value| text_of(value)) {
        out.push(MenuEntry { id: node.id, label });
    }
    for child in &node.children {
        if let Some(child) = as_node(child) {
            collect_entries(&child, out);
        }
    }
}

/// 把 `av` 里一个元素还原成节点。手工拆而不是再 `deserialize` 一次：
/// 手里已经是解出来的值，没有原始字节可以重新解。
fn as_node(value: &OwnedValue) -> Option<MenuNode> {
    let structure = value.downcast_ref::<&Structure>().ok()?;
    let fields = structure.fields();
    let id = fields.first().and_then(|field| i32::try_from(field).ok())?;
    let mut props = std::collections::HashMap::new();
    if let Ok(dict) = fields
        .get(1)
        .context("菜单节点缺少属性字典")
        .and_then(|field| field.downcast_ref::<&Dict>().map_err(Into::into))
    {
        for (key, value) in dict.iter() {
            if let (Some(key), Ok(value)) = (text_of(key), OwnedValue::try_from(value.clone())) {
                props.insert(key, value);
            }
        }
    }
    let children = fields
        .get(2)
        .and_then(|field| field.downcast_ref::<&Array>().ok())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| OwnedValue::try_from(value.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Some(MenuNode {
        id,
        props,
        children,
    })
}
