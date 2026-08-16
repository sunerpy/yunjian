use std::sync::Arc;

use yunjian_ai::{AppreciationProvider, AppreciationRequest, KeyStore};
use yunjian_core::{
    CorpusConfig, CorpusHandle, PoemDetailRequest, TextSearchRequest, VoiceSessionConfig,
};
use yunjian_mobile::{
    BindingVerdict, MobileFacade, ReciteStartRequest, ReciteSubmitRequest, VoiceSessionRequest,
};
use yunjian_recite::Scheduler;
use yunjian_voice::session::{Demonstrator, Listener};

fn assert_search_group(facade: &MobileFacade, request: TextSearchRequest) {
    let _ = facade.search_text(request);
}

fn assert_detail_group(facade: &MobileFacade, request: PoemDetailRequest) {
    let _ = facade.poem_detail(request);
}

async fn assert_ai_group(facade: &MobileFacade, request: AppreciationRequest) {
    let _ = facade.appreciate(request.clone()).await;
    let _ = facade.appreciate_stream(request).await;
}

fn assert_recitation_group(
    facade: &MobileFacade,
    start: ReciteStartRequest,
    submit: ReciteSubmitRequest,
) {
    let _ = facade.recite_start(start);
    let _ = facade.recite_submit(submit);
    let _ = facade.recite_due();
}

fn assert_voice_group<D, L>(
    facade: &MobileFacade,
    demonstrator: D,
    listener: L,
    request: VoiceSessionRequest,
) where
    D: Demonstrator + Send + 'static,
    L: Listener + Send + 'static,
{
    let handle = facade.voice_session_start(demonstrator, listener, request);
    if let Ok(handle) = handle {
        let _ = MobileFacade::voice_session_next_event(&handle, 0);
        MobileFacade::voice_session_cancel(&handle);
        MobileFacade::voice_session_close(&handle);
    }
}

fn assert_keystore_group(facade: &MobileFacade) {
    let _ = facade.keystore_status("fixture");
    let _ = facade.keystore_set("fixture", "secret");
    let _ = facade.keystore_delete("fixture");
}

fn assert_corpus_group(config: &CorpusConfig, corpus: &CorpusHandle) {
    let _ = MobileFacade::corpus_materialize(config, &mut |_| {});
    let _ = MobileFacade::corpus_status(corpus);
}

#[test]
fn facade_exposes_every_required_feature_group() {
    let _ = assert_search_group as fn(&MobileFacade, TextSearchRequest);
    let _ = assert_detail_group as fn(&MobileFacade, PoemDetailRequest);
    let _ = assert_ai_group;
    let _ = assert_recitation_group as fn(&MobileFacade, ReciteStartRequest, ReciteSubmitRequest);
    let _ = assert_voice_group::<FixtureDemonstrator, FixtureListener>;
    let _ = assert_keystore_group as fn(&MobileFacade);
    let _ = assert_corpus_group as fn(&CorpusConfig, &CorpusHandle);

    let constructor: fn(
        CorpusHandle,
        Arc<dyn AppreciationProvider>,
        Scheduler,
        KeyStore,
        VoiceSessionConfig,
    ) -> MobileFacade = MobileFacade::new;
    let _ = constructor;
}

#[test]
fn binding_branch_matches_the_recorded_three_state_verdict() {
    let report = include_str!("../../../docs/reports/mobile-spike.md");
    let recorded = if report.contains("verdict: `tauri_mobile`") {
        BindingVerdict::TauriMobile
    } else if report.contains("verdict: `uniffi_native`") {
        BindingVerdict::UniffiNative
    } else if report.contains("verdict: `undetermined`") {
        BindingVerdict::Undetermined
    } else {
        panic!("mobile spike 报告缺少可识别 verdict");
    };

    assert_eq!(yunjian_mobile::BINDING_VERDICT, recorded);

    // 这里断言的是「已构建的 binding **不得与裁决矛盾**」，而不是「必须恰好等于裁决那一对」。
    //
    // 两者的差别在 2026-08-16 才第一次显现：门禁产出真裁决（`uniffi_native`）之前，
    // 裁决是 `undetermined`，「与裁决一致」和「两个都没建」恰好同义，于是原先那条
    // `assert_eq!(built, expected)` 看起来足够。裁决落地后出现了第三种合法状态——
    // **已定选型、尚未落地**：裁决由 todo 68 的门禁产出，构建 binding 分支是 todo 69 的工作，
    // 两件事之间必然有一段时间差。原断言会把这段时间判成失败，而唯一能让它变绿的做法是
    // 把 `UNIFFI_NATIVE_BINDING` 提前写成 `true`——那是伪造一个不存在的构建产物。
    //
    // 守卫要拦的两件事一件没放过：**建错分支**（裁决说 uniffi 却建了 tauri，或反之）
    // 与**无裁决就开建**。放行的只有「还没建」。
    let (tauri_built, uniffi_built) = (
        yunjian_mobile::TAURI_MOBILE_BINDING,
        yunjian_mobile::UNIFFI_NATIVE_BINDING,
    );
    assert!(
        !(tauri_built && uniffi_built),
        "不得同时构建两个 binding 分支：方案要求只实现一个"
    );
    match recorded {
        BindingVerdict::TauriMobile => {
            assert!(!uniffi_built, "裁决是 tauri_mobile，却构建了 UniFFI 分支")
        }
        BindingVerdict::UniffiNative => assert!(
            !tauri_built,
            "裁决是 uniffi_native，却构建了 Tauri mobile 分支"
        ),
        BindingVerdict::Undetermined => assert!(
            !tauri_built && !uniffi_built,
            "裁决尚未产出时不得构建任何 binding 分支"
        ),
    }
}

/// 裁决已定而分支未建时，`Cargo.toml` 必须仍然干净。
///
/// 这条把「尚未落地」从一句解释变成一条可执行断言：只要两个 binding 常量都是 `false`，
/// manifest 里就不该出现任何 binding 依赖或 feature。todo 69 落地时两处会一起变，
/// 于是「常量说没建、manifest 里却已经有 uniffi」这种半截状态无法悄悄存在。
#[test]
fn a_pending_binding_branch_leaves_the_manifest_free_of_shell_dependencies() {
    if yunjian_mobile::TAURI_MOBILE_BINDING || yunjian_mobile::UNIFFI_NATIVE_BINDING {
        return;
    }
    let manifest = include_str!("../Cargo.toml");
    let parsed: toml::Value = toml::from_str(manifest).expect("解析 mobile manifest");
    for shell in ["uniffi", "tauri"] {
        assert!(
            parsed
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .is_none_or(|table| !table.contains_key(shell)),
            "两个 binding 常量都是 false，manifest 却依赖 {shell}：半截落地"
        );
        assert!(
            parsed
                .get("features")
                .and_then(toml::Value::as_table)
                .is_none_or(|table| !table.contains_key(shell)),
            "两个 binding 常量都是 false，manifest 却声明 {shell} feature：半截落地"
        );
    }
}

struct FixtureDemonstrator;

impl Demonstrator for FixtureDemonstrator {
    fn demonstrate(
        &mut self,
        _line: &str,
    ) -> Result<yunjian_voice::session::Demonstration, yunjian_voice::VoiceError> {
        unreachable!("编译期完整性断言不会运行语音装置")
    }
}

struct FixtureListener;

impl Listener for FixtureListener {
    fn listen(
        &mut self,
        _line: &str,
        _plan: &yunjian_voice::recognize::RecognitionPlan,
    ) -> Result<yunjian_voice::session::LineTake, yunjian_voice::VoiceError> {
        unreachable!("编译期完整性断言不会运行语音装置")
    }
}
