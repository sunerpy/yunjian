use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, Resource, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};

const MAIN_WINDOW: &str = "main";
const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(5);
const DISABLE_STARTUP_CHECK: &str = "YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK";

// 整个平台键枚举按 `desktop` 门住，而不是给它补一个「移动端」变体。两条理由：
//
// 1. 发布清单只为三个桌面目标签 updater 产物，移动端根本没有可下载的更新包——
//    补一个变体就得给它编一个平台键，而那个键在 `latest.json` 里不存在。
// 2. `tests/updater_contract.rs` 断言映射键**精确等于**发布清单的三个键。在 Android
//    上把枚举整体去掉，三个 `=> "..."` 字面量在源码里保持原样，契约不必为了移动端放宽。
//
// 于是 Android 上该类型不存在，取而代之的是下面 `updater()` 里那条走
// `sanitize_update_error("platform")` 的分支——它复用了既有的
// 「当前平台的更新包尚未发布」文案，不新增一种用户可见语义。
#[cfg(desktop)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateTarget {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    WindowsX86_64Nsis,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    DarwinAarch64,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    LinuxX86_64,
}

#[cfg(desktop)]
impl UpdateTarget {
    fn current() -> Self {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        return Self::WindowsX86_64Nsis;
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        return Self::DarwinAarch64;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return Self::LinuxX86_64;
    }

    fn as_str(self) -> &'static str {
        match self {
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            Self::WindowsX86_64Nsis => "windows-x86_64-nsis",
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::DarwinAarch64 => "darwin-aarch64",
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::LinuxX86_64 => "linux-x86_64",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct UpdateInfo {
    current_version: String,
    version: String,
    notes: Option<String>,
    published_at: Option<String>,
}

impl From<&Update> for UpdateInfo {
    fn from(update: &Update) -> Self {
        Self {
            current_version: update.current_version.clone(),
            version: update.version.clone(),
            notes: update.body.clone(),
            published_at: update.date.map(|date| date.to_string()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum UpdateEvent {
    Started { total: Option<u64> },
    Progress { downloaded: u64, total: Option<u64> },
    Downloaded,
    Installing,
    Finished,
}

fn sanitize_update_error(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("request")
        || normalized.contains("network")
        || normalized.contains("connect")
        || normalized.contains("dns")
        || normalized.contains("timed out")
    {
        "无法连接更新服务。云笺已按系统代理模式发起请求，请检查系统代理、网络连接后重试。"
            .to_owned()
    } else if normalized.contains("signature") || normalized.contains("minisign") {
        "更新包签名验证失败，已拒绝安装。请等待发布方修复更新文件。".to_owned()
    } else if normalized.contains("platform") || normalized.contains("target") {
        "当前平台的更新包尚未发布，请稍后重试。".to_owned()
    } else {
        "检查或安装更新失败，请稍后重试。".to_owned()
    }
}

#[cfg(desktop)]
fn updater<R: Runtime>(app: &AppHandle<R>) -> Result<tauri_plugin_updater::Updater, String> {
    app.updater_builder()
        .target(UpdateTarget::current().as_str())
        .build()
        .map_err(|error| sanitize_update_error(&error.to_string()))
}

// 移动端没有 updater 产物，所以这里在**构造 updater 之前**就失败，而不是拿一个空平台键
// 去请求 `latest.json` 后拿 404 当「无更新」——两者对用户是不同的事实。
#[cfg(not(desktop))]
fn updater<R: Runtime>(_app: &AppHandle<R>) -> Result<tauri_plugin_updater::Updater, String> {
    Err(sanitize_update_error("platform"))
}

async fn check<R: Runtime>(app: &AppHandle<R>) -> Result<Option<UpdateInfo>, String> {
    let update = updater(app)?
        .check()
        .await
        .map_err(|error| sanitize_update_error(&error.to_string()))?;
    let Some(update) = update else {
        return Ok(None);
    };
    let info = UpdateInfo::from(&update);
    let update = Arc::new(update);
    update.close();
    Ok(Some(info))
}

#[tauri::command]
pub(crate) async fn update_check<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<UpdateInfo>, String> {
    check(&app).await
}

#[cfg(target_os = "linux")]
fn ensure_installable_linux_bundle() -> Result<(), String> {
    if std::env::var_os("APPIMAGE").is_none() {
        return Err(
            "Linux 自动更新仅支持 AppImage；deb/rpm 安装请使用系统包管理器完成升级。".to_owned(),
        );
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn ensure_installable_linux_bundle() -> Result<(), String> {
    Ok(())
}

async fn install<R: Runtime>(
    app: &AppHandle<R>,
    on_event: Channel<UpdateEvent>,
) -> Result<bool, String> {
    ensure_installable_linux_bundle()?;
    let update = updater(app)?
        .check()
        .await
        .map_err(|error| sanitize_update_error(&error.to_string()))?;
    let Some(update) = update else {
        return Ok(false);
    };

    let _ = on_event.send(UpdateEvent::Started { total: None });
    let downloaded = AtomicU64::new(0);
    let progress = on_event.clone();
    let bytes = update
        .download(
            |chunk_length, total| {
                let downloaded = downloaded.fetch_add(chunk_length as u64, Ordering::Relaxed)
                    + chunk_length as u64;
                let _ = progress.send(UpdateEvent::Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|error| sanitize_update_error(&error.to_string()))?;
    let _ = on_event.send(UpdateEvent::Downloaded);
    let _ = on_event.send(UpdateEvent::Installing);
    update
        .install(bytes)
        .map_err(|error| sanitize_update_error(&error.to_string()))?;
    let update = Arc::new(update);
    update.close();
    let _ = on_event.send(UpdateEvent::Finished);
    Ok(true)
}

#[tauri::command]
pub(crate) async fn update_download_and_install<R: Runtime>(
    app: AppHandle<R>,
    on_event: Channel<UpdateEvent>,
) -> Result<bool, String> {
    install(&app, on_event).await
}

fn native_progress_channel<R: Runtime>(app: AppHandle<R>) -> Channel<UpdateEvent> {
    Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            return Ok(());
        };
        let event = serde_json::from_str::<UpdateEvent>(&json)?;
        if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
            let title = match event {
                UpdateEvent::Started { .. } => "云笺 · 正在下载更新".to_owned(),
                UpdateEvent::Progress { downloaded, total } => total.map_or_else(
                    || format!("云笺 · 已下载 {} MiB", downloaded / 1_048_576),
                    |total| {
                        let percent = downloaded.saturating_mul(100) / total.max(1);
                        format!("云笺 · 下载更新 {percent}%")
                    },
                ),
                UpdateEvent::Downloaded | UpdateEvent::Installing => {
                    "云笺 · 正在安装更新".to_owned()
                }
                UpdateEvent::Finished => "云笺".to_owned(),
            };
            window.set_title(&title)?;
        }
        Ok(())
    })
}

fn show_error<R: Runtime>(app: &AppHandle<R>, message: String) {
    app.dialog()
        .message(message)
        .title("云笺更新失败")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

fn prompt_update<R: Runtime>(app: AppHandle<R>, info: UpdateInfo) {
    let version = info.version.clone();
    let dialog_app = app.clone();
    app.dialog()
        .message(format!("云笺 {version} 已发布。是否现在下载并安装？"))
        .title("发现新版本")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "现在更新".to_owned(),
            "稍后".to_owned(),
        ))
        .show(move |install_now| {
            if !install_now {
                return;
            }
            let app = dialog_app.clone();
            tauri::async_runtime::spawn(async move {
                let progress = native_progress_channel(app.clone());
                if let Err(message) = install(&app, progress).await {
                    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                        let _ = window.set_title("云笺");
                    }
                    show_error(&app, message);
                }
            });
        });
}

pub(crate) fn start_delayed_check<R: Runtime>(app: AppHandle<R>) {
    if std::env::var_os(DISABLE_STARTUP_CHECK).is_some() {
        tracing::debug!("启动更新检查已由运行环境禁用");
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_CHECK_DELAY).await;
        match check(&app).await {
            Ok(Some(info)) => prompt_update(app, info),
            Ok(None) => tracing::debug!("当前已是最新版本"),
            Err(message) => {
                tracing::warn!(error = %message, "启动更新检查失败");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_errors_are_readable_and_do_not_echo_proxy_credentials() {
        let message = sanitize_update_error(
            "error sending request for url (https://proxy-user:proxy-secret@example.invalid)",
        );
        assert!(message.contains("系统代理"));
        assert!(!message.contains("proxy-user"));
        assert!(!message.contains("proxy-secret"));
    }

    #[cfg(desktop)]
    #[test]
    fn current_target_is_one_of_the_release_manifest_keys() {
        assert!(!UpdateTarget::current().as_str().is_empty());
    }
}
