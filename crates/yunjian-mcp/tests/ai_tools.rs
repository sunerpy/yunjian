mod common;

use async_trait::async_trait;
use common::{
    ANCHOR, EXPECTED_TOOLS_AI, Sandbox, Session, args, expected_tools_all, first_text,
    is_valid_tool_name, structured, tool_json, tool_named,
};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use yunjian_ai::{
    AiProvider, Appreciation, AppreciationCache, AppreciationProgress, AppreciationProvider,
    AppreciationRequest, AppreciationStreamItem, GeneratedPoem, PoemGenerationProvider,
    PoemGenerationRequest, ProviderId, ShippedAppreciation,
};
use yunjian_core::operation::OperationHandle;
use yunjian_core::{Error, PoemDetailRequest, Result as CoreResult};
use yunjian_mcp::YunjianServer;

#[derive(Debug)]
struct FixtureAiProvider {
    appreciate_calls: AtomicUsize,
    generate_calls: AtomicUsize,
    generated_poem: String,
    provider: ProviderId,
}

impl FixtureAiProvider {
    fn new(generated_poem: impl Into<String>) -> Self {
        Self {
            appreciate_calls: AtomicUsize::new(0),
            generate_calls: AtomicUsize::new(0),
            generated_poem: generated_poem.into(),
            provider: ProviderId::new("fixture-ai").expect("fixture provider id"),
        }
    }

    fn appreciate_calls(&self) -> usize {
        self.appreciate_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AppreciationProvider for FixtureAiProvider {
    async fn appreciate(&self, request: AppreciationRequest) -> CoreResult<Appreciation> {
        self.appreciate_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Appreciation {
            text: "fixture 赏析".to_owned(),
            model: request.model().to_owned(),
            provider: self.provider.clone(),
            generated_at: 1,
            template_version: request.template_version().to_owned(),
            grounding_digest: request.grounding_digest().to_owned(),
            usage: None,
        })
    }

    async fn appreciate_stream(
        &self,
        _request: AppreciationRequest,
    ) -> CoreResult<OperationHandle<AppreciationProgress, AppreciationStreamItem>> {
        Err(Error::ai("fixture-ai", "测试不调用流式赏析"))
    }

    fn id(&self) -> ProviderId {
        self.provider.clone()
    }
}

#[async_trait]
impl PoemGenerationProvider for FixtureAiProvider {
    async fn generate_poem(&self, request: PoemGenerationRequest) -> CoreResult<GeneratedPoem> {
        self.generate_calls.fetch_add(1, Ordering::SeqCst);
        assert!(request.prompt().contains("七言绝句"));
        assert!(request.prompt().contains("七阳"));
        Ok(GeneratedPoem {
            text: self.generated_poem.clone(),
            model: request.model().to_owned(),
            provider: self.provider.clone(),
            generated_at: 2,
            usage: None,
        })
    }
}

fn generated_fixture() -> &'static str {
    "月照松江映晚光\n风来竹径带微霜\n客心遥寄云边雁\n梦绕故园归旧乡"
}

#[tokio::test]
async fn ai_tools_complete_the_exact_five_tool_set_with_open_world_annotations() {
    let sandbox = Sandbox::new();
    let session = Session::connect(YunjianServer::new(sandbox.core())).await;
    let tools = session.tools().await;
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(names, expected_tools_all());
    assert!(names.iter().all(|name| is_valid_tool_name(name)));

    for name in EXPECTED_TOOLS_AI {
        let online = tool_json(tool_named(&tools, name));
        let annotations = &online["annotations"];
        assert_eq!(annotations["readOnlyHint"], json!(true));
        assert_eq!(annotations["destructiveHint"], json!(false));
        assert_eq!(annotations["openWorldHint"], json!(true));
        assert!(annotations.get("idempotentHint").is_none());
        assert!(online["inputSchema"].is_object());
        assert!(online["outputSchema"].is_object());
    }
    session.shutdown().await;
}

#[tokio::test]
async fn missing_key_is_a_successful_result_with_settings_guidance_for_both_tools() {
    let sandbox = Sandbox::new();
    let session = Session::connect(YunjianServer::new(sandbox.core())).await;
    let calls = [
        ("appreciate_poem", args(vec![("poem_id", json!(ANCHOR))])),
        (
            "generate_poem",
            args(vec![("form", json!("七言绝句")), ("theme", json!("思乡"))]),
        ),
    ];

    for (name, arguments) in calls {
        let result = session.call(name, arguments).await;
        assert_eq!(
            result.is_error,
            Some(false),
            "{name} 缺密钥不得成为工具错误"
        );
        let payload = structured(&result);
        let text_payload = serde_json::from_str::<serde_json::Value>(&first_text(&result))
            .expect("AI 工具 text block 应为 JSON");
        assert_eq!(text_payload, payload);
        assert_eq!(payload["status"], json!("configuration_required"));
        assert!(
            payload["message"]
                .as_str()
                .is_some_and(|message| message.contains("密钥")),
            "{name} 应说明缺少密钥：{payload}"
        );
        assert!(
            payload["settings_path"]
                .as_str()
                .is_some_and(|path| path.contains("设置") && path.contains("AI")),
            "{name} 应给出设置路径：{payload}"
        );
        assert!(
            payload["disclosure"]
                .as_str()
                .is_some_and(|note| note.contains("AI") && note.contains("未经人工审校")),
            "{name} 每个结果都必须携带未审校说明：{payload}"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn shipped_appreciation_is_provider_independent_and_skips_the_provider() {
    let sandbox = Sandbox::new();
    let core = sandbox.core();
    let detail = core
        .poem_detail(PoemDetailRequest {
            poem_id: ANCHOR.to_owned(),
        })
        .expect("读取锚定作品");
    let request = AppreciationRequest::new(detail, "fixture-model");
    let cache = Arc::new(
        AppreciationCache::open(sandbox.app_data_dir(), "mcp-fixture-v1", 20)
            .expect("打开赏析缓存"),
    );
    cache
        .insert_shipped(&ShippedAppreciation {
            stable_id: ANCHOR.to_owned(),
            template_version: request.template_version().to_owned(),
            model: "shipped-open-model".to_owned(),
            model_license: "MIT".to_owned(),
            grounding_digest: request.grounding_digest().to_owned(),
            text: "随包赏析".to_owned(),
            generated_at: 1,
        })
        .expect("写随包赏析 fixture");
    let provider = Arc::new(FixtureAiProvider::new(generated_fixture()));
    let ai: Arc<dyn AiProvider> = provider.clone();
    let server = YunjianServer::new(core).with_ai(ai, Some(cache), "fixture-model");
    let session = Session::connect(server).await;

    let result = session
        .call("appreciate_poem", args(vec![("poem_id", json!(ANCHOR))]))
        .await;
    assert_eq!(result.is_error, Some(false));
    let payload = structured(&result);
    assert_eq!(payload["status"], json!("ready"));
    assert_eq!(payload["source"], json!("shipped"));
    assert_eq!(payload["model"], json!("shipped-open-model"));
    assert_eq!(
        payload["template_version"],
        json!(request.template_version())
    );
    assert_eq!(payload["text"], json!("随包赏析"));
    assert_eq!(provider.appreciate_calls(), 0);
    assert!(
        payload["disclosure"]
            .as_str()
            .is_some_and(|note| note.contains("未经人工审校"))
    );
    session.shutdown().await;
}

#[tokio::test]
async fn generated_seven_character_quatrain_is_labelled_rhymed_and_never_persisted() {
    let sandbox = Sandbox::new();
    let core = sandbox.core();
    let corpus_before = sandbox.corpus_row_count();
    let cache = Arc::new(
        AppreciationCache::open(sandbox.app_data_dir(), "mcp-fixture-v1", 20)
            .expect("打开赏析缓存"),
    );
    let cache_before = cache.counts().expect("统计赏析缓存");
    let provider = Arc::new(FixtureAiProvider::new(generated_fixture()));
    let ai: Arc<dyn AiProvider> = provider;
    let session = Session::connect(YunjianServer::new(core).with_ai(
        ai,
        Some(Arc::clone(&cache)),
        "fixture-model",
    ))
    .await;

    let result = session
        .call(
            "generate_poem",
            args(vec![
                ("form", json!("七言绝句")),
                ("theme", json!("月夜思乡")),
                ("rhyme_book", json!("pingshui")),
                ("rhyme_group", json!("七阳")),
            ]),
        )
        .await;
    assert_eq!(result.is_error, Some(false));
    let payload = structured(&result);
    assert_eq!(payload["status"], json!("ready"));
    assert_eq!(payload["label"], json!("AI 生成，非古人作品"));
    let lines = payload["lines"].as_array().expect("lines 应为数组");
    assert_eq!(lines.len(), 4);
    assert!(
        lines
            .iter()
            .all(|line| line.as_str().expect("诗句").chars().count() == 7)
    );
    assert_eq!(payload["rhyme_book"], json!("pingshui"));
    assert_eq!(payload["rhyme_group"], json!("七阳"));
    assert_eq!(payload["rhyme_feet"], json!(["霜", "乡"]));
    assert!(
        payload["disclosure"]
            .as_str()
            .is_some_and(|note| note.contains("未经人工审校"))
    );

    assert_eq!(sandbox.corpus_row_count(), corpus_before);
    assert_eq!(cache.counts().expect("再次统计赏析缓存"), cache_before);
    session.shutdown().await;
}
