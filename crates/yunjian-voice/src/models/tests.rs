//! [`super`] 的用例。
//!
//! 除 `download` 那两条以外全部跑在**默认构建**里，因此 `make ci` 就能覆盖许可门禁、
//! 摘要校验、原子落地与降级信号。下载链用一个走真实 TCP 套接字的桩传输验证：
//! 与 `yunjian-ai` 的做法一致，刻意不引入 `wiremock`——本模块需要的只是「收一个请求、
//! 回一段固定字节」，标准库的 `TcpListener` 足够，而多一条测试依赖不划算。

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use super::{
    ALLOWED_LICENSES, DENYLIST_SOURCE, FetchProgress, MANIFEST_SOURCE, ModelCache, ModelError,
    ModelKind, ModelRole, REQUIRED_DENIED, Registry, Transport, Unpacker, cache_root, is_populated,
};
use crate::permission::{DegradeReason, Practice};

/// 仓库根。`CARGO_MANIFEST_DIR` 是 crate 目录，不是工作区根。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn shipped() -> &'static Registry {
    Registry::shipped().expect("签入的清单必须可解析")
}

// ---------------------------------------------------------------------------
// 清单与拒绝名单
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_manifest_parses_and_holds_only_allowed_licenses() {
    let registry = shipped();
    assert!(
        !registry.entries().is_empty(),
        "清单不能是空的，否则许可门禁无事可做"
    );
    for entry in registry.entries() {
        assert!(
            entry.license_allowed(),
            "{} 的许可 {} 不在允许列表",
            entry.name,
            entry.license
        );
        assert!(registry.gate(entry).is_ok(), "{} 应当通过门禁", entry.name);
        assert!(entry.size_bytes > 0, "{} 的 size_bytes 为 0", entry.name);
        assert_eq!(entry.sha256.len(), 64, "{} 的 sha256 长度不对", entry.name);
    }
    assert!(
        registry
            .entries()
            .iter()
            .any(|e| e.kind == ModelKind::Asr && e.role == ModelRole::Production),
        "至少要有一个投产 ASR"
    );
    assert!(
        registry
            .entries()
            .iter()
            .any(|e| e.kind == ModelKind::Tts && e.role == ModelRole::Production),
        "至少要有一个投产 TTS"
    );
}

/// 拒绝名单删掉一行就能放行，所以名单本身要有守卫。
#[test]
fn every_plan_named_identifier_is_still_on_the_denylist() {
    let registry = shipped();
    for required in REQUIRED_DENIED {
        assert!(
            registry.denied().iter().any(|d| d.id == required),
            "必须列入拒绝名单的 `{required}` 不见了；不允许从名单里删条目"
        );
    }
    for entry in registry.denied() {
        assert!(
            !entry.reason.trim().is_empty(),
            "拒绝条目 `{}` 没有理由；命中时报不出为什么",
            entry.id
        );
    }
}

/// 逐条构造一个用被拒名字命名的清单，断言它进不来。
///
/// 这是「方案批准的模型被换成禁用模型」的捕获点：不是只测 `matcha` 一个，而是把
/// [`REQUIRED_DENIED`] 每一条都走一遍。
#[test]
fn each_denylisted_name_is_refused_at_load_time() {
    for denied in REQUIRED_DENIED {
        let name = format!("vits-{}-probe", denied.trim_end_matches('-'));
        let registry =
            Registry::parse(&manifest_with(&name, "MIT"), DENYLIST_SOURCE).expect("探针清单可解析");
        let error = registry
            .admit(&name)
            .expect_err(&format!("`{denied}` 必须被拒"));
        let text = error.to_string();
        assert!(
            matches!(error, ModelError::Denied { .. }),
            "`{denied}` 应当命中拒绝名单而不是别的原因：{text}"
        );
        assert!(text.contains(denied), "报错要点出命中的是哪一条：{text}");
        assert_eq!(
            error.degrade_reason(),
            DegradeReason::ModelUnavailable,
            "被拒同样要能降级"
        );
    }
}

/// 许可门禁按 SPDX 判，不按「看起来像开源」判。
#[test]
fn a_gpl_licensed_model_is_refused_at_load_time() {
    let registry = Registry::parse(
        &manifest_with("probe-clean-name", "GPL-3.0"),
        DENYLIST_SOURCE,
    )
    .expect("探针清单可解析");
    let error = registry
        .admit("probe-clean-name")
        .expect_err("GPL-3.0 必须被拒");
    assert!(
        matches!(error, ModelError::LicenseRefused { .. }),
        "应当是许可被拒：{error}"
    );
    let text = error.to_string();
    assert!(text.contains("GPL-3.0"), "报错要点出是哪个许可：{text}");
    for allowed in ALLOWED_LICENSES {
        assert!(text.contains(allowed), "报错要写出允许列表：{text}");
    }
}

/// 其余非允许许可一并覆盖，含 `UNVERIFIED` 这个「许可没核实」的形态。
#[test]
fn licenses_outside_the_allow_list_are_all_refused() {
    for license in [
        "GPL-3.0",
        "AGPL-3.0",
        "CC-BY-NC-4.0",
        "UNVERIFIED",
        "other",
        "",
    ] {
        let registry =
            Registry::parse(&manifest_with("probe-clean-name", license), DENYLIST_SOURCE)
                .expect("探针清单可解析");
        assert!(
            matches!(
                registry.admit("probe-clean-name"),
                Err(ModelError::LicenseRefused { .. })
            ),
            "`{license}` 必须被拒"
        );
    }
    for license in ALLOWED_LICENSES {
        let registry =
            Registry::parse(&manifest_with("probe-clean-name", license), DENYLIST_SOURCE)
                .expect("探针清单可解析");
        assert!(
            registry.admit("probe-clean-name").is_ok(),
            "`{license}` 应当放行；正向对照缺了的话上面那组断言可能只是因为解析失败"
        );
    }
}

/// **拒绝名单先于许可**。被拒的包完全可能在清单里写着 MIT，那时必须报拒绝理由。
#[test]
fn the_denylist_outranks_a_clean_looking_license() {
    let registry = Registry::parse(&manifest_with("vits-zh-hf-doom", "MIT"), DENYLIST_SOURCE)
        .expect("探针清单可解析");
    let error = registry.admit("vits-zh-hf-doom").expect_err("必须被拒");
    assert!(
        matches!(error, ModelError::Denied { .. }),
        "写着 MIT 也要先命中拒绝名单：{error}"
    );
}

/// 换个名字挂同一个被拒 URL 也要被拦住。
#[test]
fn a_denylisted_url_is_refused_even_under_an_innocent_name() {
    let manifest = manifest_with_url(
        "probe-clean-name",
        "MIT",
        "https://example.invalid/matcha-icefall-zh-baker.tar.bz2",
    );
    let registry = Registry::parse(&manifest, DENYLIST_SOURCE).expect("探针清单可解析");
    let error = registry.admit("probe-clean-name").expect_err("必须被拒");
    assert!(
        matches!(error, ModelError::Denied { .. }),
        "拒绝匹配要同时看 url：{error}"
    );
}

#[test]
fn an_unknown_name_lists_what_is_actually_available() {
    let error = shipped()
        .admit("no-such-model")
        .expect_err("未知名字要报错");
    let text = error.to_string();
    assert!(matches!(error, ModelError::Unknown { .. }), "{text}");
    assert!(
        text.contains("sherpa-onnx-whisper-tiny"),
        "报错要列出可用的名字：{text}"
    );
}

#[test]
fn an_unknown_schema_version_is_refused_rather_than_best_effort_parsed() {
    let bumped = MANIFEST_SOURCE.replace("schema_version = 1", "schema_version = 2");
    let error = Registry::parse(&bumped, DENYLIST_SOURCE).expect_err("未知版本必须拒绝");
    assert!(
        matches!(error, ModelError::Manifest { .. }),
        "应当是清单错误：{error}"
    );
    assert!(
        error.to_string().contains("放弃校验"),
        "报错要说清为什么不尽力解析：{error}"
    );
}

#[test]
fn an_empty_denylist_is_refused_because_it_would_pass_everything() {
    let error = Registry::parse(MANIFEST_SOURCE, "## 拒绝清单\n\n没有条目。\n")
        .expect_err("空拒绝名单必须拒绝");
    assert!(matches!(error, ModelError::Manifest { .. }), "{error}");
}

// ---------------------------------------------------------------------------
// 署名：licenses/ 必须覆盖清单
// ---------------------------------------------------------------------------

#[test]
fn licenses_directory_holds_a_file_for_every_manifest_entry() {
    let dir = repo_root().join("licenses");
    for entry in shipped().entries() {
        let attribution = dir.join(entry.attribution_file());
        let shipped_bytes = std::fs::read(&attribution)
            .unwrap_or_else(|error| panic!("{} 缺署名文件：{error}", attribution.display()));
        let evidence = repo_root().join(&entry.license_file);
        let evidence_bytes = std::fs::read(&evidence)
            .unwrap_or_else(|error| panic!("{} 读不到：{error}", evidence.display()));
        assert_eq!(
            shipped_bytes,
            evidence_bytes,
            "{} 的署名副本必须与 {} 逐字节一致；不一致意味着分发的许可原文与经过 \
             verify-models 校验的证据是两份东西",
            attribution.display(),
            entry.license_file
        );
    }
}

#[test]
fn the_licenses_directory_has_no_files_beyond_the_manifest() {
    let dir = repo_root().join("licenses");
    let expected: Vec<String> = shipped()
        .entries()
        .iter()
        .map(super::ModelEntry::attribution_file)
        .collect();
    for found in std::fs::read_dir(&dir).expect("licenses/ 必须存在") {
        let found = found.expect("读目录项");
        let name = found.file_name().to_string_lossy().into_owned();
        if name == "README.md" {
            continue;
        }
        assert!(
            expected.contains(&name),
            "licenses/{name} 不对应清单里任何条目；多出来的许可原文会让署名范围含义不明"
        );
    }
}

// ---------------------------------------------------------------------------
// 缓存路径与降级信号
// ---------------------------------------------------------------------------

#[test]
fn every_failure_degrades_to_typed_practice_with_a_specific_message() {
    let cases = [
        ModelError::Unknown {
            name: "x".to_owned(),
            known: vec!["y".to_owned()],
        },
        ModelError::Denied {
            name: "x".to_owned(),
            matched: "matcha-icefall-zh-baker".to_owned(),
            reason: "非商用".to_owned(),
        },
        ModelError::LicenseRefused {
            name: "x".to_owned(),
            license: "GPL-3.0".to_owned(),
        },
        ModelError::Absent {
            name: "x".to_owned(),
            dir: PathBuf::from("/nope"),
            next: "下一步".to_owned(),
        },
        ModelError::ChecksumMismatch {
            name: "x".to_owned(),
            expected: "a".to_owned(),
            actual: "b".to_owned(),
        },
        ModelError::SizeMismatch {
            name: "x".to_owned(),
            expected: 1,
            actual: 2,
        },
        ModelError::Download {
            name: "x".to_owned(),
            url: "https://example.invalid/x".to_owned(),
            detail: "断线".to_owned(),
        },
        ModelError::Unpack {
            name: "x".to_owned(),
            detail: "坏包".to_owned(),
        },
        ModelError::Io {
            path: PathBuf::from("/nope"),
            detail: "只读".to_owned(),
        },
        ModelError::Manifest {
            detail: "语法".to_owned(),
        },
    ];
    for error in cases {
        let rendered = error.to_string();
        let practice = error.practice();
        assert!(
            practice.is_typed(),
            "{rendered} 必须降级到打字练习而不是别的结果"
        );
        assert_eq!(
            practice.reason(),
            Some(DegradeReason::ModelUnavailable),
            "{rendered}"
        );
        let Practice::Typed { message, .. } = &practice else {
            unreachable!("上一条断言已排除 Voice")
        };
        assert!(
            message.contains(rendered.trim_end_matches('。')),
            "降级消息要包含具体原因，否则用户只看到一句通用文案：{message}"
        );
        assert!(
            message.contains("models fetch"),
            "降级消息要给出恢复动作：{message}"
        );
    }
}

/// 缺失且无下载能力时是降级信号，不是错误对话框。
#[test]
fn a_missing_model_without_network_yields_the_typed_fallback() {
    let sandbox = Sandbox::new("absent");
    let error = sandbox
        .cache()
        .ensure_with("sherpa-onnx-whisper-tiny", None, None, &mut |_| {})
        .expect_err("空缓存里必须报缺失");
    assert!(
        matches!(error, ModelError::Absent { .. }),
        "应当是缺失而不是别的失败：{error}"
    );
    let practice = error.practice();
    assert!(practice.is_typed(), "必须给出打字练习");
    assert_eq!(practice.reason(), Some(DegradeReason::ModelUnavailable));
    drop(sandbox);
}

#[test]
fn cache_paths_are_relative_to_the_cache_root_and_discover_follows_the_env() {
    let sandbox = Sandbox::new("paths");
    let cache = sandbox.cache();
    assert_eq!(cache.model_dir("m"), sandbox.root().join("m"));
    assert_eq!(
        cache.archive_path("m"),
        sandbox.root().join("archives").join("m.tar.bz2"),
        "归档路径必须落在缓存内，否则 .gitignore 盖不住它"
    );
    assert_eq!(
        ModelCache::discover().root(),
        cache_root(),
        "默认构造必须走 cache_root 的解析顺序，识别器与下载器才会看同一个目录"
    );
    drop(sandbox);
}

#[test]
fn an_empty_directory_does_not_count_as_present() {
    let sandbox = Sandbox::new("empty-dir");
    let dir = sandbox.cache().model_dir("sherpa-onnx-whisper-tiny");
    std::fs::create_dir_all(&dir).expect("建空目录");
    assert!(!is_populated(&dir), "空目录不算就位");
    assert!(!sandbox.cache().is_present("sherpa-onnx-whisper-tiny"));
    let error = sandbox
        .cache()
        .ensure_with("sherpa-onnx-whisper-tiny", None, None, &mut |_| {})
        .expect_err("空目录必须仍报缺失");
    assert!(matches!(error, ModelError::Absent { .. }), "{error}");
    drop(sandbox);
}

/// 已就位时**一个网络请求都不发**。
#[test]
fn ensure_model_on_a_cached_model_performs_no_network_call() {
    let sandbox = Sandbox::new("cached");
    let dir = sandbox.cache().model_dir("sherpa-onnx-whisper-tiny");
    std::fs::create_dir_all(&dir).expect("建模型目录");
    std::fs::write(dir.join("tiny-tokens.txt"), b"0 <blk>\n").expect("放一个文件让目录非空");

    let counting = CountingTransport::default();
    let resolved = sandbox
        .cache()
        .ensure_with(
            "sherpa-onnx-whisper-tiny",
            Some(&counting),
            Some(&RefusingUnpacker),
            &mut |_| {},
        )
        .expect("已缓存的模型必须直接返回");
    assert_eq!(resolved, dir);
    assert_eq!(
        counting.calls(),
        0,
        "已缓存时不得发起任何传输；解包器也刻意用一个只会失败的实现，\
         它没被调用本身就是第二条证据"
    );
    drop(sandbox);
}

// ---------------------------------------------------------------------------
// 下载、校验与原子落地
// ---------------------------------------------------------------------------

/// 摘要不符时中止，且**不留下任何文件**。
#[test]
fn a_checksum_mismatch_aborts_and_leaves_no_file() {
    let sandbox = Sandbox::new("mismatch");
    let name = "sherpa-onnx-whisper-tiny";
    let entry = shipped().find(name).expect("清单里有它");
    let wrong = vec![0_u8; usize::try_from(entry.size_bytes).expect("测试用尺寸")];

    let transport = StubTransport::new(wrong);
    let error = sandbox
        .cache()
        .ensure_with(name, Some(&transport), Some(&RefusingUnpacker), &mut |_| {})
        .expect_err("摘要不符必须失败");
    assert!(
        matches!(error, ModelError::ChecksumMismatch { .. }),
        "应当是摘要不符：{error}"
    );

    assert!(
        !sandbox.cache().archive_path(name).exists(),
        "摘要不符时归档不得以最终名字留下"
    );
    let leftovers = list_dir(&sandbox.root().join("archives"));
    assert!(
        leftovers.is_empty(),
        "摘要不符时归档目录必须是空的，临时文件也不许留：{leftovers:?}"
    );
    assert!(
        !sandbox.cache().model_dir(name).exists(),
        "失败时不该建出模型目录"
    );
    drop(sandbox);
}

/// 字节数不符先于摘要报出来，因为它几乎总是「下载被截断」。
#[test]
fn a_truncated_download_reports_size_rather_than_checksum() {
    let sandbox = Sandbox::new("truncated");
    let name = "sherpa-onnx-whisper-tiny";
    let transport = StubTransport::new(b"too short".to_vec());
    let error = sandbox
        .cache()
        .ensure_with(name, Some(&transport), Some(&RefusingUnpacker), &mut |_| {})
        .expect_err("截断必须失败");
    assert!(
        matches!(error, ModelError::SizeMismatch { .. }),
        "截断应当报字节数：{error}"
    );
    assert!(
        list_dir(&sandbox.root().join("archives")).is_empty(),
        "不留文件"
    );
    drop(sandbox);
}

/// 超出清单字节数的响应必须在写盘途中就被拦下，而不是等摘要校验。
#[test]
fn a_response_longer_than_the_manifest_is_cut_off_mid_write() {
    let sandbox = Sandbox::new("overlong");
    let name = "sherpa-onnx-whisper-tiny";
    let entry = shipped().find(name).expect("清单里有它");
    let overlong = vec![
        7_u8;
        usize::try_from(entry.size_bytes)
            .expect("测试用尺寸")
            .saturating_add(4096)
    ];

    let transport = StubTransport::new(overlong);
    let error = sandbox
        .cache()
        .ensure_with(name, Some(&transport), Some(&RefusingUnpacker), &mut |_| {})
        .expect_err("超长响应必须失败");
    assert!(
        matches!(error, ModelError::Download { .. }),
        "应当在传输阶段被上限拦下：{error}"
    );
    assert!(
        error.to_string().contains("超过清单记录"),
        "报错要说清是上限拦的：{error}"
    );
    assert!(
        list_dir(&sandbox.root().join("archives")).is_empty(),
        "不留文件"
    );
    drop(sandbox);
}

/// 摘要相符时原子落地，解包后返回目录，并报出进度。
#[test]
fn a_matching_download_lands_atomically_and_reports_progress() {
    let sandbox = Sandbox::new("happy");
    let (name, bytes, digest) = synthetic_entry_bytes();
    let manifest = manifest_with_digest(&name, "MIT", &digest, bytes.len() as u64);
    let registry = Registry::parse(&manifest, DENYLIST_SOURCE).expect("探针清单可解析");
    let entry = registry.admit(&name).expect("探针条目应当放行");

    let transport = StubTransport::new(bytes.clone());
    let landed = sandbox.cache().archive_path(&name);
    let mut seen = Vec::new();
    super::download_verified(entry, &landed, &transport, &mut |event| seen.push(event))
        .expect("摘要相符必须成功");

    assert!(landed.is_file(), "归档必须以最终名字落地");
    assert_eq!(std::fs::read(&landed).expect("读归档"), bytes);
    assert_eq!(
        list_dir(&sandbox.root().join("archives")),
        vec![format!("{name}.tar.bz2")],
        "落地后只该有最终文件，临时文件必须已清掉"
    );
    assert!(
        seen.contains(&FetchProgress::Verified),
        "进度里必须出现「已校验」：{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|e| matches!(e, FetchProgress::Downloading { .. })),
        "进度里必须出现下载事件：{seen:?}"
    );
    drop(sandbox);
}

/// 归档已在本地但摘要被改坏时，仍然拒绝加载。**绝不加载未校验的下载。**
#[test]
fn a_cached_archive_with_a_bad_digest_is_refused_without_touching_the_network() {
    let sandbox = Sandbox::new("stale-archive");
    let name = "sherpa-onnx-whisper-tiny";
    let entry = shipped().find(name).expect("清单里有它");
    let archive = sandbox.cache().archive_path(name);
    std::fs::create_dir_all(archive.parent().expect("有父目录")).expect("建归档目录");
    std::fs::write(
        &archive,
        vec![1_u8; usize::try_from(entry.size_bytes).expect("测试用尺寸")],
    )
    .expect("写一个字节数对但内容错的归档");

    let counting = CountingTransport::default();
    let error = sandbox
        .cache()
        .ensure_with(name, Some(&counting), Some(&RefusingUnpacker), &mut |_| {})
        .expect_err("坏归档必须被拒");
    assert!(
        matches!(error, ModelError::ChecksumMismatch { .. }),
        "应当是摘要不符：{error}"
    );
    assert_eq!(
        counting.calls(),
        0,
        "本地已有归档时不该重新下载；这条断言同时证明校验发生在加载之前"
    );
    drop(sandbox);
}

/// 解包走同目录临时目录再整体改名，失败时不留半个模型目录。
#[test]
fn a_failing_unpack_leaves_no_partial_model_directory() {
    let sandbox = Sandbox::new("unpack-fail");
    let (name, bytes, digest) = synthetic_entry_bytes();
    let manifest = manifest_with_digest(&name, "MIT", &digest, bytes.len() as u64);
    let registry = Registry::parse(&manifest, DENYLIST_SOURCE).expect("探针清单可解析");
    let entry = registry.admit(&name).expect("探针条目应当放行");
    let archive = sandbox.cache().archive_path(&name);
    super::download_verified(entry, &archive, &StubTransport::new(bytes), &mut |_| {})
        .expect("先把归档落地");

    let dir = sandbox.cache().model_dir(&name);
    let error = super::unpack_atomically(&name, &RefusingUnpacker, &archive, &dir)
        .expect_err("解包必须失败");
    assert!(matches!(error, ModelError::Unpack { .. }), "{error}");
    assert!(!dir.exists(), "失败时不得留下模型目录");
    let leftovers: Vec<String> = list_dir(sandbox.root())
        .into_iter()
        .filter(|n| n != "archives")
        .collect();
    assert!(leftovers.is_empty(), "临时目录必须清掉：{leftovers:?}");
    drop(sandbox);
}

/// 解包器把内容放进临时目录后，整体改名成模型目录；顶层同名目录会被下潜一层。
#[test]
fn a_successful_unpack_flattens_the_top_level_directory() {
    let sandbox = Sandbox::new("unpack-ok");
    let name = "sherpa-onnx-whisper-tiny";
    let archive = sandbox.cache().archive_path(name);
    std::fs::create_dir_all(archive.parent().expect("有父目录")).expect("建归档目录");
    std::fs::write(&archive, b"not really an archive").expect("桩解包器不读它");

    let dir = sandbox.cache().model_dir(name);
    super::unpack_atomically(name, &NestingUnpacker { name }, &archive, &dir)
        .expect("解包应当成功");
    let landed = dir.join("tokens.txt");
    assert!(
        landed.is_file(),
        "顶层同名目录要被下潜，最终结构是 <cache>/<name>/tokens.txt，实际 {}",
        landed.display()
    );
    drop(sandbox);
}

/// 归档里没有顶层同名目录时用临时目录本身，不能因此失败。
#[test]
fn an_archive_without_a_top_level_directory_still_lands() {
    let sandbox = Sandbox::new("unpack-flat");
    let name = "sherpa-onnx-whisper-tiny";
    let archive = sandbox.cache().archive_path(name);
    std::fs::create_dir_all(archive.parent().expect("有父目录")).expect("建归档目录");
    std::fs::write(&archive, b"not really an archive").expect("桩解包器不读它");

    let dir = sandbox.cache().model_dir(name);
    super::unpack_atomically(name, &FlatUnpacker, &archive, &dir).expect("解包应当成功");
    assert!(dir.join("tokens.txt").is_file());
    drop(sandbox);
}

#[test]
fn remove_cached_deletes_both_the_directory_and_the_archive() {
    let sandbox = Sandbox::new("remove");
    let name = "sherpa-onnx-whisper-tiny";
    let dir = sandbox.cache().model_dir(name);
    std::fs::create_dir_all(&dir).expect("建模型目录");
    std::fs::write(dir.join("tokens.txt"), b"x").expect("写文件");
    let archive = sandbox.cache().archive_path(name);
    std::fs::create_dir_all(archive.parent().expect("有父目录")).expect("建归档目录");
    std::fs::write(&archive, b"x").expect("写归档");

    let removed = sandbox.cache().remove(name).expect("删除应当成功");
    assert!(removed.dir && removed.archive, "两者都该删掉：{removed:?}");
    assert!(!dir.exists() && !archive.exists());

    let again = sandbox.cache().remove(name).expect("重复删除不算失败");
    assert!(again.is_empty(), "第二次没有东西可删：{again:?}");
    drop(sandbox);
}

/// 被拒的模型如果已经躺在缓存里，最该允许的操作就是把它删掉。
#[test]
fn remove_cached_does_not_apply_the_license_gate() {
    let sandbox = Sandbox::new("remove-denied");
    assert!(
        matches!(
            sandbox.cache().remove("no-such-model"),
            Err(ModelError::Unknown { .. })
        ),
        "未知名字仍要报错"
    );
    assert!(
        sandbox.cache().remove("sherpa-onnx-whisper-tiny").is_ok(),
        "清单里的名字可删"
    );
    drop(sandbox);
}

#[test]
fn statuses_cover_every_manifest_entry_and_flag_refusals() {
    let sandbox = Sandbox::new("statuses");
    let rows = sandbox.cache().statuses().expect("列出状态");
    assert_eq!(rows.len(), shipped().entries().len(), "每个条目一行");
    for row in &rows {
        assert!(
            row.refused.is_none(),
            "{} 不该被拒：{:?}",
            row.name,
            row.refused
        );
        assert!(!row.unpacked, "空缓存里不该有已解包的模型");
        assert!(!row.archived, "空缓存里不该有归档");
        assert!(
            row.attribution.starts_with(&row.name),
            "{}",
            row.attribution
        );
    }
    drop(sandbox);
}

/// 走真实 TCP 套接字，证明「下载 → 校验 → 落地」不是只在桩上成立。
#[test]
fn the_download_chain_works_over_a_real_socket() {
    let sandbox = Sandbox::new("socket");
    let (name, bytes, digest) = synthetic_entry_bytes();
    let manifest = manifest_with_digest(&name, "MIT", &digest, bytes.len() as u64);
    let registry = Registry::parse(&manifest, DENYLIST_SOURCE).expect("探针清单可解析");
    let entry = registry.admit(&name).expect("探针条目应当放行");

    let (base, probe) = spawn_probe(bytes.clone());
    let transport = LoopbackTransport { base };
    let landed = sandbox.cache().archive_path(&name);
    let mut seen = Vec::new();
    super::download_verified(entry, &landed, &transport, &mut |event| seen.push(event))
        .expect("经真实套接字下载应当成功");
    let requested = probe.join().expect("回收探针");

    assert!(
        requested.starts_with("GET "),
        "探针必须真的收到一个 HTTP 请求：{requested}"
    );
    assert_eq!(
        std::fs::read(&landed).expect("读归档"),
        bytes,
        "经套接字拿到的字节必须与服务端发的一致"
    );
    assert!(seen.contains(&FetchProgress::Verified), "{seen:?}");
    drop(sandbox);
}

// ---------------------------------------------------------------------------
// 测试脚手架
// ---------------------------------------------------------------------------

/// 一个自己清理的空缓存目录。
///
/// 每条用例一个独立目录，因此这些用例可以并行——缓存根是 [`ModelCache`] 的构造入参，
/// 不需要动任何进程全局状态。
struct Sandbox {
    cache: ModelCache,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "yunjian-models-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("沙盒目录可创建");
        Self {
            cache: ModelCache::at(root),
        }
    }

    fn cache(&self) -> &ModelCache {
        &self.cache
    }

    fn root(&self) -> &Path {
        self.cache.root()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.cache.root());
    }
}

fn list_dir(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 一份只有一个条目的最小清单。
fn manifest_with(name: &str, license: &str) -> String {
    manifest_with_url(
        name,
        license,
        &format!("https://example.invalid/{name}.tar.bz2"),
    )
}

fn manifest_with_url(name: &str, license: &str, url: &str) -> String {
    manifest_entry(name, license, url, &"0".repeat(64), 1024)
}

fn manifest_with_digest(name: &str, license: &str, digest: &str, size: u64) -> String {
    manifest_entry(
        name,
        license,
        &format!("https://example.invalid/{name}.tar.bz2"),
        digest,
        size,
    )
}

fn manifest_entry(name: &str, license: &str, url: &str, digest: &str, size: u64) -> String {
    format!(
        r#"schema_version = 1

[[model]]
name = "{name}"
kind = "tts"
role = "smoke"
url = "{url}"
sha256 = "{digest}"
size_bytes = {size}
license = "{license}"
license_url = "https://example.invalid/LICENSE"
license_rev = "{rev}"
license_file = "models/licenses/vits-melo-tts-zh_en.LICENSE"
license_sha256 = "{digest}"
license_evidence = "package_license"
underlying_work = "测试用探针条目"
verified_at = "2026-08-12"
"#,
        rev = "a".repeat(40),
    )
}

/// 一段自洽的探针字节与它的真实摘要。
fn synthetic_entry_bytes() -> (String, Vec<u8>, String) {
    let bytes: Vec<u8> = (0..4096_u32).map(|i| (i % 251) as u8).collect();
    let digest = sha256_hex(&bytes);
    ("probe-clean-name".to_owned(), bytes, digest)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 回一段固定字节的桩传输。
struct StubTransport {
    body: Vec<u8>,
}

impl StubTransport {
    const fn new(body: Vec<u8>) -> Self {
        Self { body }
    }
}

impl Transport for StubTransport {
    fn fetch(
        &self,
        _url: &str,
        sink: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u64, String> {
        let total = self.body.len() as u64;
        let mut done = 0_u64;
        for chunk in self.body.chunks(1024) {
            sink.write_all(chunk).map_err(|error| error.to_string())?;
            done = done.saturating_add(chunk.len() as u64);
            progress(done, total);
        }
        Ok(done)
    }
}

/// 只数调用次数、永不成功的传输。用来断言「没发起网络请求」。
#[derive(Default)]
struct CountingTransport {
    calls: std::sync::atomic::AtomicUsize,
}

impl CountingTransport {
    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Transport for CountingTransport {
    fn fetch(
        &self,
        _url: &str,
        _sink: &mut dyn std::io::Write,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u64, String> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err("本用例不该发起下载".to_owned())
    }
}

/// 永远失败的解包器。出现在「不该走到解包」的用例里，它没被调用本身就是断言。
struct RefusingUnpacker;

impl Unpacker for RefusingUnpacker {
    fn unpack(&self, _archive: &Path, _into: &Path) -> Result<(), String> {
        Err("本用例不该走到解包".to_owned())
    }
}

/// 造出「顶层一个同名目录」这种真实归档结构。
struct NestingUnpacker<'a> {
    name: &'a str,
}

impl Unpacker for NestingUnpacker<'_> {
    fn unpack(&self, _archive: &Path, into: &Path) -> Result<(), String> {
        let nested = into.join(self.name);
        std::fs::create_dir_all(&nested).map_err(|error| error.to_string())?;
        std::fs::write(nested.join("tokens.txt"), b"0 <blk>\n").map_err(|error| error.to_string())
    }
}

/// 造出「没有顶层目录」的归档结构。
struct FlatUnpacker;

impl Unpacker for FlatUnpacker {
    fn unpack(&self, _archive: &Path, into: &Path) -> Result<(), String> {
        std::fs::write(into.join("tokens.txt"), b"0 <blk>\n").map_err(|error| error.to_string())
    }
}

/// 走本地回环的真实 HTTP/1.1 GET。
///
/// 清单里的 URL 必须是 https（那条约束由 `xtask verify-models` 守着，不该为测试放宽），
/// 所以这里不改清单，而是把请求发给探针端口：验证的是「真的走了套接字、字节逐一落盘」。
struct LoopbackTransport {
    base: String,
}

impl Transport for LoopbackTransport {
    fn fetch(
        &self,
        url: &str,
        sink: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u64, String> {
        let path = url
            .rsplit_once('/')
            .map_or("/", |(_, last)| last)
            .to_owned();
        let mut stream = TcpStream::connect(&self.base).map_err(|error| error.to_string())?;
        write!(
            stream,
            "GET /{path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.base
        )
        .map_err(|error| error.to_string())?;

        let mut reader = BufReader::new(stream);
        let mut total = 0_u64;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("探针在发完头之前就断了".to_owned());
            }
            if line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                total = value.trim().parse().unwrap_or(0);
            }
        }

        let mut buffer = vec![0_u8; 1024];
        let mut done = 0_u64;
        loop {
            let read = std::io::Read::read(&mut reader, &mut buffer).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            sink.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            done = done.saturating_add(read as u64);
            progress(done, total);
        }
        Ok(done)
    }
}

/// 起一个只应答一次的最小 HTTP 端点，把收到的请求行交回调用方。
fn spawn_probe(body: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定探针端口");
    let addr = listener.local_addr().expect("取探针地址").to_string();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("接受探针连接");
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).expect("读请求行");
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
        }
        let mut stream = stream;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
        request_line.trim().to_owned()
    });
    (addr, handle)
}
