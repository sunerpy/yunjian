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

    let (tauri_built, uniffi_built) = (
        yunjian_mobile::TAURI_MOBILE_BINDING,
        yunjian_mobile::UNIFFI_NATIVE_BINDING,
    );
    assert!(
        !(tauri_built && uniffi_built),
        "不得同时构建两个 binding 分支：方案要求只实现一个"
    );
    match recorded {
        BindingVerdict::TauriMobile => assert!(
            tauri_built && !uniffi_built,
            "裁决是 tauri_mobile，构建状态却不唯一匹配"
        ),
        BindingVerdict::UniffiNative => assert!(
            !tauri_built && uniffi_built,
            "裁决是 uniffi_native，构建状态却不唯一匹配"
        ),
        BindingVerdict::Undetermined => assert!(
            !tauri_built && !uniffi_built,
            "裁决尚未产出时不得构建任何 binding 分支"
        ),
    }
}

#[test]
fn built_binding_state_matches_manifest_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    let parsed: toml::Value = toml::from_str(manifest).expect("解析 mobile manifest");
    let dependencies = parsed
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("mobile manifest 应有 dependencies 表");
    let features = parsed
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("mobile manifest 应有 features 表");
    assert_eq!(
        yunjian_mobile::UNIFFI_NATIVE_BINDING,
        dependencies.contains_key("uniffi") && features.contains_key("uniffi")
    );
    assert_eq!(
        yunjian_mobile::TAURI_MOBILE_BINDING,
        dependencies.contains_key("tauri") && features.contains_key("tauri")
    );
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
