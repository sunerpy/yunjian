use serde_json::Value;
use std::path::PathBuf;

const PLATFORM_KEYS: [&str; 3] = ["windows-x86_64-nsis", "darwin-aarch64", "linux-x86_64"];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source(relative: &str) -> String {
    let path = crate_dir().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()))
}

fn config() -> Value {
    serde_json::from_str(&source("tauri.conf.json")).expect("tauri.conf.json 必须是合法 JSON")
}

#[test]
fn updater_configuration_requires_signed_artifacts_and_a_stable_endpoint() {
    let config = config();
    assert_eq!(
        config["bundle"]["createUpdaterArtifacts"],
        Value::Bool(true),
        "bundle.createUpdaterArtifacts 必须开启"
    );
    let updater = &config["plugins"]["updater"];
    let pubkey = updater["pubkey"].as_str().unwrap_or_default();
    assert!(
        !pubkey.is_empty() && !pubkey.contains("PRIVATE"),
        "updater.pubkey 必须配置公开签名密钥且不得包含私钥"
    );
    let endpoints = updater["endpoints"]
        .as_array()
        .expect("缺少 updater.endpoints");
    assert_eq!(endpoints.len(), 1, "更新端点必须只有一个稳定来源");
    assert_eq!(
        endpoints[0],
        "https://github.com/sunerpy/yunjian/releases/latest/download/latest.json"
    );
}

#[test]
fn updater_uses_a_closed_platform_key_set() {
    let updater = source("src/updater.rs");
    assert!(
        updater.contains("enum UpdateTarget"),
        "平台必须用封闭枚举建模，不能在请求处拼裸字符串"
    );
    for key in PLATFORM_KEYS {
        assert_eq!(
            updater.matches(key).count(),
            1,
            "平台键 `{key}` 必须恰好由枚举映射声明一次"
        );
    }
    let mut mapped_keys = updater
        .lines()
        .filter_map(|line| line.split_once("=> \"").map(|(_, value)| value))
        .filter_map(|value| value.strip_suffix("\","))
        .collect::<Vec<_>>();
    mapped_keys.sort_unstable();
    let mut allowed_keys = PLATFORM_KEYS.to_vec();
    allowed_keys.sort_unstable();
    assert_eq!(
        mapped_keys, allowed_keys,
        "平台映射必须精确等于发布清单支持的三个键"
    );
    assert!(
        updater.contains("target(UpdateTarget::current().as_str())"),
        "检查更新时必须使用封闭类型产生的真实平台键"
    );
}

#[test]
fn updater_commands_stream_progress_and_release_check_only_resources() {
    let updater = source("src/updater.rs");
    assert!(
        updater.contains("on_event: Channel<UpdateEvent>"),
        "下载进度必须通过必填 ipc::Channel 传递"
    );
    assert!(
        !updater.contains("Option<Channel<UpdateEvent>>"),
        "Channel 不得包装成 Option，否则不实现 CommandArg"
    );
    assert!(
        updater.contains("update.close()"),
        "只检查不安装时必须关闭 Update 资源"
    );
    assert!(
        updater.contains("tokio::time::sleep") && updater.contains("STARTUP_CHECK_DELAY"),
        "启动检查必须延迟执行，不能阻塞首屏"
    );
    assert!(
        !updater.contains("spawn_blocking") && !updater.contains("Runtime::new"),
        "更新网络 Future 必须直接 await，不能放入阻塞池或新建 runtime"
    );
}

#[test]
fn updater_enables_system_proxy_and_sanitizes_network_errors() {
    let manifest = source("Cargo.toml");
    assert!(
        manifest.contains("tauri-plugin-updater") && manifest.contains("system-proxy"),
        "updater 依赖必须显式启用 reqwest/system-proxy"
    );
    let updater = source("src/updater.rs");
    assert!(
        updater.contains("系统代理") && updater.contains("sanitize_update_error"),
        "裸网络错误必须映射为说明系统代理模式的可读消息"
    );
}

#[test]
fn updater_plugin_and_startup_check_are_wired_into_the_builder() {
    let lib = source("src/lib.rs");
    assert!(
        lib.contains("mod updater;"),
        "桌面外壳必须声明 updater 模块"
    );
    assert!(
        lib.contains("tauri_plugin_updater::Builder::new().build()"),
        "Builder 必须注册签名 updater 插件"
    );
    assert!(
        lib.contains("updater::start_delayed_check(app.handle().clone())"),
        "setup 必须调度非阻塞的启动更新检查"
    );
}

#[test]
fn background_startup_check_failure_never_opens_a_modal_dialog() {
    let updater = source("src/updater.rs");
    let delayed_check = updater
        .split_once("pub(crate) fn start_delayed_check")
        .map(|(_, body)| body)
        .expect("updater.rs 必须实现 start_delayed_check");
    let delayed_check = delayed_check
        .split_once("#[cfg(test)]")
        .map_or(delayed_check, |(body, _)| body);

    assert!(
        delayed_check.contains("tracing::warn!"),
        "后台启动检查失败仍要写日志，不能静默吞掉"
    );
    assert!(
        !delayed_check.contains("show_error("),
        "后台启动检查失败不得弹原生模态框：它会抢走首屏焦点并吞掉标题栏、输入框的真实交互。\
         用户主动点击检查或安装时仍可显示错误"
    );
}

#[test]
fn startup_check_has_an_acceptance_isolation_switch() {
    let updater = source("src/updater.rs");
    assert!(
        updater.contains("YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK")
            && updater.contains("var_os(DISABLE_STARTUP_CHECK)"),
        "桌面交互验收必须能隔离外部更新服务，避免提示框抢走被测窗口的输入"
    );
}
