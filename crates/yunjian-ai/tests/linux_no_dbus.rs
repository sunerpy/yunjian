//! 失败场景：Linux 上没有 Secret Service 时的降级与界面文案。
//!
//! 断言的是**端到端的诚实链条**：Secret Service 不可用 → 退到内核 keyutils → 报告说
//! `login_session` → 由该报告派生的设置界面文案里没有「系统钥匙串」。
//!
//! `DBUS_SESSION_BUS_ADDRESS` 的清除放在子进程里做，而不是在测试线程里改进程环境：
//! 后者在 Rust 2024 里是 `unsafe`，而且会污染同一进程内并行跑的其他测试。

#![cfg(target_os = "linux")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use secrecy::{ExposeSecret, SecretString};
use yunjian_ai::keystore::{
    Backend, KeyStore, KeyStoreConfig, Persistence, Protection, StorageReport,
    install_default_store,
};

const CHILD_MARKER: &str = "YUNJIAN_KEYSTORE_NO_DBUS_CHILD";
const REPORT_PATH: &str = "YUNJIAN_KEYSTORE_REPORT_PATH";

fn report_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "yunjian-keystore-no-dbus-{}.json",
        std::process::id()
    ))
}

#[test]
fn without_secret_service_falls_back_to_keyutils_and_ui_never_says_system_keychain() {
    if std::env::var(CHILD_MARKER).is_ok() {
        return;
    }

    let exe = std::env::current_exe().expect("拿不到测试二进制路径");
    let dump = report_path();
    fs::remove_file(&dump).ok();

    let output = Command::new(&exe)
        .args([
            "--exact",
            "child_probes_store_selection_without_dbus",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env(REPORT_PATH, &dump)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output()
        .expect("启动子进程失败");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "子进程（无 D-Bus）失败：\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    let dumped = fs::read_to_string(&dump).expect("子进程没有写出报告");
    fs::remove_file(&dump).ok();

    assert!(
        dumped.contains(r#""backend":"keyutils""#),
        "应当退到 keyutils：{dumped}"
    );
    assert!(
        dumped.contains(r#""persistence":"login_session""#),
        "keyutils 必须报 login_session：{dumped}"
    );
    assert!(
        !dumped.contains("系统钥匙串"),
        "由该报告派生的界面文案不得出现「系统钥匙串」：{dumped}"
    );
    assert!(
        dumped.contains("重启"),
        "界面文案必须点明重启后失效：{dumped}"
    );
}

/// 由上面那条测试以清空 `DBUS_SESSION_BUS_ADDRESS` 的子进程身份调用。
#[test]
#[ignore = "由 without_secret_service_falls_back_to_keyutils_and_ui_never_says_system_keychain 以子进程调用"]
fn child_probes_store_selection_without_dbus() {
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err(),
        "场景没有真的生效：DBUS_SESSION_BUS_ADDRESS 仍然有值"
    );

    let keychain = install_default_store()
        .expect("Secret Service 不可用时应当退到 keyutils，而不是完全没有钥匙串");

    assert_eq!(
        keychain.backend(),
        Backend::Keyutils,
        "无 D-Bus 时不该选到 Secret Service"
    );
    assert_eq!(keychain.persistence(), Persistence::LoginSession);

    let report = StorageReport {
        backend: keychain.backend(),
        persistence: keychain.persistence(),
        protection: Protection::OsEncrypted,
        location: keychain.location().to_string(),
    };
    let summary = report.settings_summary();
    assert!(
        !summary.contains("系统钥匙串"),
        "诚实链条断了：keyutils 的界面文案宣称了系统钥匙串：{summary}"
    );

    // 选中不等于可用：真的往这一层写一次、读回来、再删掉，证明降级后的存储确实能干活。
    let store = KeyStore::open(KeyStoreConfig::default()).expect("打开密钥存储失败");
    assert_eq!(store.tier_report().backend, Backend::Keyutils);
    let account = format!("no-dbus-probe-{}", std::process::id());
    store
        .set(&account, &SecretString::from("sk-TESTKEY123".to_string()))
        .expect("keyutils 写入失败");
    let found = store.get(&account).expect("keyutils 读取失败");
    assert_eq!(
        found.secret().map(|s| s.expose_secret()),
        Some("sk-TESTKEY123"),
        "keyutils 取回的密钥与写入的不一致"
    );
    store.delete(&account).expect("keyutils 删除失败");
    assert!(
        store
            .get(&account)
            .expect("keyutils 读取失败")
            .needs_reprompt(),
        "删除后必须给出重新索要信号"
    );

    let dumped = serde_json::json!({
        "backend": report.backend.as_str(),
        "persistence": report.persistence.as_str(),
        "protection": report.protection.as_str(),
        "location": report.location,
        "settings_summary": summary,
        "round_trip": "set/get/delete 全部通过",
    });
    let path = std::env::var(REPORT_PATH).expect("父进程没有给出报告路径");
    fs::write(
        path,
        serde_json::to_string(&dumped).expect("序列化报告失败"),
    )
    .expect("写出报告失败");
}
