//! [`super`] 的测试。
//!
//! 平台后端用 `keyring_core::mock::Store` 替身，这样 `get`/`set`/`delete` 的语义、
//! 报告内容与降级链都能在任何平台上被断言，而不依赖本机是否装了钥匙串守护进程。
//! 真机后端的逐平台结论另行记录，不在此处伪装成通过。

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use keyring_core::api::CredentialStoreApi;
use keyring_core::{CredentialStore, mock};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::*;

const FAKE_KEY: &str = "sk-TESTKEY123";

fn mock_store() -> Arc<CredentialStore> {
    mock::Store::new().expect("mock store 建立失败")
}

fn keychain_store(backend: Backend) -> KeyStore {
    KeyStore::from_credential_store(mock_store(), DEFAULT_SERVICE, backend, "mock 凭据 store")
}

/// 每次调用给出一个本进程内唯一的临时目录，避免并行测试互踩。
fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "yunjian-keystore-{}-{}-{seq}",
        std::process::id(),
        tag
    ));
    fs::create_dir_all(&dir).expect("建立临时目录失败");
    dir
}

#[test]
fn round_trip_get_set_delete() {
    let store = keychain_store(Backend::SecretService);

    let before = store.get("openai").expect("读取失败");
    assert!(
        before.needs_reprompt(),
        "还没写入就该报「需要重新索要」：{before:?}"
    );

    let set = store
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("写入失败");
    assert_eq!(set.backend, Backend::SecretService);

    let found = store.get("openai").expect("读取失败");
    assert!(!found.needs_reprompt(), "写入后不该再要求重新索要");
    assert_eq!(
        found.secret().map(|s| s.expose_secret()),
        Some(FAKE_KEY),
        "取回的密钥与写入的不一致"
    );
    assert_eq!(found.report().backend, Backend::SecretService);

    let deleted = store.delete("openai").expect("删除失败");
    assert_eq!(deleted.backend, Backend::SecretService);

    let after = store.get("openai").expect("读取失败");
    assert_eq!(after.report().backend, Backend::Absent, "删除后应当不存在");
}

#[test]
fn missing_key_is_absent_plus_reprompt_not_an_error() {
    let store = keychain_store(Backend::Keyutils);

    let lookup = store.get("从未设置过").expect("「未找到」不得作为错误返回");

    assert!(lookup.needs_reprompt(), "必须给出重新索要信号");
    assert!(lookup.secret().is_none());
    let report = lookup.report();
    assert_eq!(report.backend, Backend::Absent);
    assert_eq!(report.backend.as_str(), "absent");
    assert_eq!(report.persistence, Persistence::None);
}

/// keyutils 上「密钥不存在」是重启后的**常态**，不是异常。上游文档要求调用方
/// 「prepare for `Entry::get_password` to fail」——这条测试就是那句话在本项目的落点。
#[test]
fn keyring_no_entry_error_becomes_absent_rather_than_propagating() {
    let store = mock_store();
    let entry = store
        .build(DEFAULT_SERVICE, "openai", None)
        .expect("建条目失败");
    entry.set_password(FAKE_KEY).expect("写入失败");
    let cred: &mock::Cred = entry.as_any().downcast_ref().expect("downcast 失败");
    cred.set_error(KeyringError::NoEntry);

    let keystore = KeyStore::from_credential_store(
        store.clone(),
        DEFAULT_SERVICE,
        Backend::Keyutils,
        "Linux 内核 keyutils",
    );
    // mock 的注入错误只对下一次调用生效，所以这里让 keystore 用同一个 store 重新建条目
    // 并触发那一次被注入的失败。
    let injected = store
        .build(DEFAULT_SERVICE, "openai", None)
        .expect("建条目失败");
    let injected_cred: &mock::Cred = injected.as_any().downcast_ref().expect("downcast 失败");
    injected_cred.set_error(KeyringError::NoEntry);

    let lookup = keystore.get("openai").expect("NoEntry 必须被吞成 Absent");
    assert!(lookup.needs_reprompt());
    assert_eq!(lookup.report().backend, Backend::Absent);
}

/// **本模块存在的理由。** keyutils 与 Secret Service 都是「操作系统钥匙串」，若持久性
/// 也一样，设置界面就会对一个重启即失效的密钥宣称持久存储。
#[test]
fn keyutils_persistence_maps_to_login_session_not_persistent() {
    let mapped = Persistence::from_credential_persistence(CredentialPersistence::UntilReboot);

    assert_eq!(mapped, Persistence::LoginSession);
    assert_ne!(mapped, Persistence::Persistent);
    assert_eq!(mapped.as_str(), "login_session");
}

#[test]
fn persistence_mapping_covers_every_upstream_variant_conservatively() {
    assert_eq!(
        Persistence::from_credential_persistence(CredentialPersistence::UntilDelete),
        Persistence::Persistent
    );
    assert_eq!(
        Persistence::from_credential_persistence(CredentialPersistence::UntilLogout),
        Persistence::LoginSession
    );
    assert_eq!(
        Persistence::from_credential_persistence(CredentialPersistence::ProcessOnly),
        Persistence::ProcessOnly
    );
    assert_eq!(
        Persistence::from_credential_persistence(CredentialPersistence::EntryOnly),
        Persistence::ProcessOnly
    );
    // `CredentialPersistence` 是 `#[non_exhaustive]` 的。未知取值必须落到非持久档，
    // 否则上游新增一档就会让界面替未知语义做出持久承诺。
    assert_eq!(
        Persistence::from_credential_persistence(CredentialPersistence::Unspecified),
        Persistence::LoginSession
    );
}

/// 诚实链条：报告 → 界面文案。keyutils 的文案里不得出现「系统钥匙串」。
#[test]
fn settings_string_for_keyutils_is_not_system_keychain() {
    let keyutils = StorageReport {
        backend: Backend::Keyutils,
        persistence: Persistence::LoginSession,
        protection: Protection::OsEncrypted,
        location: "Linux 内核 keyutils".to_string(),
    };

    let summary = keyutils.settings_summary();

    assert!(
        !summary.contains("系统钥匙串"),
        "keyutils 只活到重启，文案不得宣称系统钥匙串：{summary}"
    );
    assert!(
        summary.contains("重启"),
        "文案必须点明重启后失效：{summary}"
    );
    assert!(
        summary.contains("重新输入"),
        "文案必须告诉用户届时要重新输入：{summary}"
    );
}

#[test]
fn settings_string_says_system_keychain_only_when_persistent_and_os_encrypted() {
    for backend in [
        Backend::SecretService,
        Backend::WindowsCredential,
        Backend::AppleKeychain,
        Backend::AndroidKeystore,
    ] {
        let report = StorageReport {
            backend,
            persistence: Persistence::Persistent,
            protection: Protection::OsEncrypted,
            location: "某个真钥匙串".to_string(),
        };
        assert!(
            report.settings_summary().contains("系统钥匙串"),
            "{backend:?} 是持久且系统加密的，应当显示系统钥匙串"
        );
    }

    // 任何一个维度掉档，「系统钥匙串」就不许出现。
    for (persistence, protection) in [
        (Persistence::LoginSession, Protection::OsEncrypted),
        (Persistence::ProcessOnly, Protection::ProcessMemory),
        (Persistence::Persistent, Protection::Plaintext),
        (Persistence::None, Protection::OsEncrypted),
    ] {
        let report = StorageReport {
            backend: Backend::SecretService,
            persistence,
            protection,
            location: "降档的后端".to_string(),
        };
        assert!(
            !report.settings_summary().contains("系统钥匙串"),
            "{persistence:?}/{protection:?} 不配「系统钥匙串」：{}",
            report.settings_summary()
        );
    }
}

#[test]
fn session_memory_reports_process_only() {
    let store = KeyStore::session_memory(DEFAULT_SERVICE);

    let report = store
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("写入失败");

    assert_eq!(report.backend, Backend::SessionMemory);
    assert_eq!(report.backend.as_str(), "session_memory");
    assert_eq!(report.persistence, Persistence::ProcessOnly);
    assert_eq!(report.persistence.as_str(), "process_only");
    assert_eq!(report.protection, Protection::ProcessMemory);
    assert_eq!(report.protection.as_str(), "process_memory");
    assert!(report.plaintext_warning().is_none(), "内存层不是明文存储");

    let found = store.get("openai").expect("读取失败");
    assert_eq!(found.secret().map(|s| s.expose_secret()), Some(FAKE_KEY));

    store.delete("openai").expect("删除失败");
    assert!(
        store.get("openai").expect("读取失败").needs_reprompt(),
        "删除后应当要求重新索要"
    );
}

/// `SecretString` 的 zeroize 保证分两截，两截都断言：
///
/// 1. 类型层面：`SecretString` 实现 [`ZeroizeOnDrop`]（编译期约束）；
/// 2. 行为层面：`SecretBox` 的 `Drop` 真的调了内容的 `zeroize`（用一个会计数的替身观测）。
///
/// 刻意不去读已释放内存来「看见零」——那需要 `unsafe`，且结果不可靠。
#[test]
fn session_secret_zeroizes_on_drop() {
    fn require_zeroize_on_drop<T: ZeroizeOnDrop>() {}
    require_zeroize_on_drop::<SecretString>();

    #[derive(Clone, Default)]
    struct Tracked {
        calls: Arc<AtomicUsize>,
        bytes: Vec<u8>,
    }

    impl Zeroize for Tracked {
        fn zeroize(&mut self) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.bytes.zeroize();
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    {
        let boxed = SecretBox::new(Box::new(Tracked {
            calls: Arc::clone(&calls),
            bytes: FAKE_KEY.as_bytes().to_vec(),
        }));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "drop 之前不该被 zeroize");
        drop(boxed);
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "SecretBox 的 Drop 必须 zeroize 内容"
    );
}

#[test]
fn plaintext_file_refuses_to_write_without_opt_in() {
    let dir = temp_dir("no-opt-in");
    let path = dir.join(PLAINTEXT_FILE_NAME);
    let store = KeyStore::plaintext_file(DEFAULT_SERVICE, &path, false);

    let err = store
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect_err("未 opt-in 必须拒绝写入");

    let rendered = format!("{err}");
    assert!(
        rendered.contains("allow_plaintext_file"),
        "报错要指出缺的是哪个开关：{rendered}"
    );
    assert!(!path.exists(), "被拒绝时不得留下任何文件");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn plaintext_file_reports_plaintext_and_warns() {
    let dir = temp_dir("opt-in");
    let path = dir.join(PLAINTEXT_FILE_NAME);
    let store = KeyStore::plaintext_file(DEFAULT_SERVICE, &path, true);

    let report = store
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("opt-in 后应当写入成功");

    assert_eq!(report.backend, Backend::PlaintextFile);
    assert_eq!(report.backend.as_str(), "plaintext_file");
    assert_eq!(report.protection, Protection::Plaintext);
    assert_eq!(report.protection.as_str(), "plaintext");
    assert_eq!(report.persistence, Persistence::Persistent);

    let warning = report.plaintext_warning().expect("明文层必须给出警告");
    assert!(warning.contains("未加密"), "警告要说清没有加密：{warning}");
    assert!(
        !report.settings_summary().contains("系统钥匙串"),
        "明文文件不得冒充系统钥匙串"
    );

    let found = store.get("openai").expect("读取失败");
    assert_eq!(found.secret().map(|s| s.expose_secret()), Some(FAKE_KEY));

    store.delete("openai").expect("删除失败");
    assert!(store.get("openai").expect("读取失败").needs_reprompt());

    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn plaintext_file_is_mode_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("mode");
    // 刻意把文件放进一个**尚不存在**的子目录：目录的 0700 由生产代码在创建时设置，
    // 让测试自己先 mkdir 就只会量到 umask，量不到被测行为。
    let nested = dir.join("yunjian");
    let path = nested.join(PLAINTEXT_FILE_NAME);
    let store = KeyStore::plaintext_file(DEFAULT_SERVICE, &path, true);

    store
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("写入失败");

    let mode = fs::metadata(&path)
        .expect("读取元数据失败")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "明文文件必须是 0600，实际 {mode:o}");

    let dir_mode = fs::metadata(&nested)
        .expect("读取目录元数据失败")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        dir_mode, 0o700,
        "新建的明文密钥目录必须是 0700，实际 {dir_mode:o}"
    );

    fs::remove_dir_all(&dir).ok();
}

/// `OpenOptions::mode` 只在本次调用真的创建了文件时生效。接管一个既存的宽松文件时，
/// 必须由显式 `set_permissions` 把它收紧，否则权限会静默保持 0644。
#[cfg(unix)]
#[test]
fn plaintext_file_tightens_permissions_on_a_preexisting_loose_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("tighten");
    let path = dir.join(PLAINTEXT_FILE_NAME);
    fs::write(&path, "").expect("预置文件失败");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("预置权限失败");

    KeyStore::plaintext_file(DEFAULT_SERVICE, &path, true)
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("写入失败");

    let mode = fs::metadata(&path)
        .expect("读取元数据失败")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "既存的 0644 文件必须被收紧到 0600，实际 {mode:o}"
    );

    fs::remove_dir_all(&dir).ok();
}

/// Windows 上 POSIX mode bits 没有意义，改判 ACL：文件必须只授权当前用户，
/// 且不得保留继承来的 `BUILTIN\Users`。
#[cfg(windows)]
#[test]
fn plaintext_file_acl_is_restricted_to_current_user() {
    use std::process::Command;

    let dir = temp_dir("acl");
    let path = dir.join(PLAINTEXT_FILE_NAME);
    KeyStore::plaintext_file(DEFAULT_SERVICE, &path, true)
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("写入失败");

    let out = Command::new("icacls")
        .arg(&path)
        .output()
        .expect("执行 icacls 失败");
    let acl = String::from_utf8_lossy(&out.stdout);
    let user = std::env::var("USERNAME").expect("USERNAME 未设置");

    assert!(acl.contains(&user), "ACL 里应当有当前用户：{acl}");
    assert!(
        !acl.contains("BUILTIN\\Users"),
        "继承来的 BUILTIN\\Users 必须被 /inheritance:r 丢掉：{acl}"
    );

    fs::remove_dir_all(&dir).ok();
}

/// 密钥不得出现在任何渲染结果里。沿用 todo 4 落地的 `Error::Ai` 双层脱敏。
#[test]
fn keystore_errors_never_render_the_key() {
    let err = keystore_error(
        "读取",
        &KeyringError::Invalid(
            "password".to_string(),
            format!("upstream rejected api_key={FAKE_KEY}"),
        ),
    );

    for rendered in [format!("{err}"), format!("{err:?}")] {
        assert!(
            !rendered.contains("TESTKEY"),
            "渲染结果泄露了密钥：{rendered}"
        );
    }
}

#[test]
fn debug_output_of_keystore_and_report_carries_no_secret() {
    let store = KeyStore::session_memory(DEFAULT_SERVICE);
    store
        .set("openai", &SecretString::from(FAKE_KEY.to_string()))
        .expect("写入失败");

    let rendered = format!("{store:?}");
    assert!(
        !rendered.contains("TESTKEY"),
        "KeyStore 的 Debug 泄露了密钥：{rendered}"
    );

    let lookup = store.get("openai").expect("读取失败");
    let rendered = format!("{:?}", lookup.report());
    assert!(
        !rendered.contains("TESTKEY"),
        "StorageReport 的 Debug 泄露了密钥：{rendered}"
    );
}

/// 四个字段的取值域就是界面与 FFI 的契约。改动这些字符串等于改协议。
#[test]
fn report_field_vocabularies_are_stable() {
    let backends = [
        (Backend::SecretService, "secret_service"),
        (Backend::Keyutils, "keyutils"),
        (Backend::WindowsCredential, "windows_credential"),
        (Backend::AppleKeychain, "apple_keychain"),
        (Backend::AndroidKeystore, "android_keystore"),
        (Backend::SessionMemory, "session_memory"),
        (Backend::PlaintextFile, "plaintext_file"),
        (Backend::Absent, "absent"),
    ];
    for (value, token) in backends {
        assert_eq!(value.as_str(), token);
        assert_eq!(
            serde_json::to_string(&value).expect("序列化失败"),
            format!("\"{token}\""),
            "序列化取值必须与 as_str 一致"
        );
    }

    for (value, token) in [
        (Persistence::Persistent, "persistent"),
        (Persistence::LoginSession, "login_session"),
        (Persistence::ProcessOnly, "process_only"),
        (Persistence::None, "none"),
    ] {
        assert_eq!(value.as_str(), token);
        assert_eq!(
            serde_json::to_string(&value).expect("序列化失败"),
            format!("\"{token}\"")
        );
    }

    for (value, token) in [
        (Protection::OsEncrypted, "os_encrypted"),
        (Protection::ProcessMemory, "process_memory"),
        (Protection::Plaintext, "plaintext"),
    ] {
        assert_eq!(value.as_str(), token);
        assert_eq!(
            serde_json::to_string(&value).expect("序列化失败"),
            format!("\"{token}\"")
        );
    }
}

/// 真机 Windows 凭据管理器：走完整的平台探测路径 [`install_default_store`]，并真的
/// 往里写一次、读回、删掉。
///
/// 与 keyutils 那条对称，断言的是**相反**的一侧：Windows 凭据管理器是持久的，所以它
/// 必须报 `persistent` 并且界面文案**应当**说「系统钥匙串」——诚实链条不是「永远不说
/// 系统钥匙串」，而是「只在配得上的时候说」。
#[cfg(windows)]
#[test]
#[allow(
    clippy::print_stderr,
    reason = "跳过原因必须可见，否则这条测试在没有凭据管理器的机器上会静默通过，看起来像验过了"
)]
fn real_windows_credential_store_round_trips_and_reports_persistent() {
    let Some(keychain) = install_default_store() else {
        eprintln!(
            "跳过 real_windows_credential_store_round_trips_and_reports_persistent：\
             本机没有可用的凭据管理器"
        );
        return;
    };

    assert_eq!(keychain.backend(), Backend::WindowsCredential);
    assert_eq!(
        keychain.persistence(),
        Persistence::Persistent,
        "Windows 凭据管理器是持久存储"
    );
    let summary = keychain.report().settings_summary();
    assert!(
        summary.contains("系统钥匙串"),
        "持久且系统加密的后端应当显示系统钥匙串：{summary}"
    );

    let account = format!("windows-ci-probe-{}", std::process::id());
    let store = KeyStore::open(KeyStoreConfig::default()).expect("打开密钥存储失败");
    assert_eq!(store.tier_report().backend, Backend::WindowsCredential);

    store
        .set(&account, &SecretString::from(FAKE_KEY.to_string()))
        .expect("凭据管理器写入失败");
    let found = store.get(&account).expect("凭据管理器读取失败");
    assert_eq!(
        found.secret().map(|s| s.expose_secret()),
        Some(FAKE_KEY),
        "取回的密钥与写入的不一致"
    );
    store.delete(&account).expect("凭据管理器删除失败");
    assert!(
        store.get(&account).expect("读取失败").needs_reprompt(),
        "删除后必须给出重新索要信号"
    );
}

/// 真机 keyutils。**只在 Linux 且 keyutils 真的可用时断言**，不可用时如实跳过而不是
/// 假装通过——本机是否有内核密钥环不是本模块能决定的事。
#[cfg(target_os = "linux")]
#[test]
#[allow(
    clippy::print_stderr,
    reason = "跳过原因必须可见，否则这条测试在无 keyutils 的机器上会静默通过，看起来像验过了"
)]
fn real_keyutils_store_reports_login_session() {
    let Ok(store) = linux_keyutils_keyring_store::Store::new() else {
        eprintln!("跳过 real_keyutils_store_reports_login_session：本机 keyutils 不可用");
        return;
    };

    let mapped = Persistence::from_credential_persistence(store.persistence());
    assert_eq!(
        mapped,
        Persistence::LoginSession,
        "真机 keyutils 必须报告 login_session"
    );
    assert_ne!(mapped, Persistence::Persistent);

    let keychain = OsKeychain::adopt(store, Backend::Keyutils, "Linux 内核 keyutils");
    assert_eq!(keychain.persistence(), Persistence::LoginSession);
    let summary = keychain.report().settings_summary();
    assert!(
        !summary.contains("系统钥匙串"),
        "真机 keyutils 的界面文案不得宣称系统钥匙串：{summary}"
    );
}
