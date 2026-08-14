use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::http::{Request as HttpRequest, Response as HttpResponse, StatusCode, header};
use tauri::ipc::Channel;
use tauri::{AppHandle, Builder, Manager, Runtime};
use yunjian_ai::{
    Appreciation, AppreciationCache, AppreciationProgress, AppreciationProvider,
    AppreciationRequest, AppreciationStreamItem, CacheHit, CacheSource,
    DEFAULT_APPRECIATION_CACHE_CAPACITY, GenAiProvider, GenAiProviderConfig, KeyStore,
    KeyStoreConfig, ProviderId, ProviderKind, PurgeScope,
};
use yunjian_core::config::{AiConfig, user_config_path};
use yunjian_core::operation::{Event, OperationHandle, cancel, next_event};
use yunjian_core::{
    Config, CorpusHandle, DictionaryLookup, DictionaryLookupRequest, MaterializationProgress,
    MetaPage, PoemDetail, PoemDetailRequest, SearchPage, TagBrowseRequest, TagSummary,
    TextSearchRequest, Yunjian,
};
use yunjian_recite::{
    AlignOp, ClozeOptions, FsrsGrade, MaskStage, PracticeMode, PracticeSession, ReviewState,
    Scheduler, SubstitutionClass, TypedScore, align, classify_substitution, grade_typed,
    review_typed_text,
};
use yunjian_voice::annotate::{Annotation, annotate_poem};
use yunjian_voice::lexicon::Poyin;
use yunjian_voice::models::ModelCache;

use crate::APP;
use yunjian_voice::permission::Practice;

use crate::voice_ipc::{VoiceRig, production_rig};

const RECITE_DATABASE_FILE: &str = "recite.db";
const AUDIO_SCHEME: &str = "yunjian-audio";

pub(crate) type IpcResult<T> = Result<T, String>;

pub(crate) struct AppState {
    config: RwLock<Config>,
    config_path: Option<PathBuf>,
    key_store: KeyStore,
    operations: OperationRegistry,
    audio: AudioStore,
    voice: RwLock<Arc<dyn VoiceRig>>,
}

impl AppState {
    fn new(config: Config) -> Self {
        let key_store = KeyStore::open(KeyStoreConfig::default()).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "无法打开系统密钥存储，退化到本进程内存");
            KeyStore::session_memory(APP)
        });
        Self {
            config: RwLock::new(config),
            config_path: user_config_path(APP),
            key_store,
            operations: OperationRegistry::default(),
            audio: AudioStore::default(),
            voice: RwLock::new(production_rig()),
        }
    }

    pub(crate) fn config(&self) -> Config {
        self.config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn replace_config(&self, config: Config) {
        *self
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
    }

    /// 当前语音装置。
    pub(crate) fn voice_rig(&self) -> Arc<dyn VoiceRig> {
        Arc::clone(
            &self
                .voice
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// 换掉语音装置。**只有测试会调它**：命令的外壳要能在一台没有模型、没有声卡的机器上
    /// 被真的调用一次，否则「权限被拒会切到打字模式并带上原因」这条断言只能靠读代码确认。
    ///
    /// `cfg` 里带 `not(windows)` 与 `voice_ipc::tests` 的门同步：那半边测试在 Windows 上整段
    /// 不编译（见 `voice_ipc/wire_tests.rs` 的模块文档），只写 `cfg(test)` 会在 Windows 上留下
    /// 一个没人调用的方法，而 `-D warnings` 会把那条 dead_code 变成一次失败。
    #[cfg(all(test, not(windows)))]
    pub(crate) fn install_voice_rig(&self, rig: Arc<dyn VoiceRig>) {
        *self
            .voice
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = rig;
    }

    /// 把一段音频交给自定义 URI 协议，返回 WebView 可取的地址。
    pub(crate) fn put_audio(&self, bytes: Vec<u8>, mime: impl Into<String>) -> String {
        self.audio.put(bytes, mime)
    }
}

trait Cancellable: Send + Sync {
    fn cancel(&self);
}

struct CoreCancellation<P, I> {
    handle: Arc<OperationHandle<P, I>>,
}

impl<P, I> Cancellable for CoreCancellation<P, I>
where
    P: Send + 'static,
    I: Send + 'static,
{
    fn cancel(&self) {
        cancel(&self.handle);
    }
}

#[derive(Default)]
struct OperationRegistry {
    entries: Mutex<HashMap<String, Arc<dyn Cancellable>>>,
}

impl OperationRegistry {
    fn insert<P, I>(&self, id: String, handle: Arc<OperationHandle<P, I>>)
    where
        P: Send + 'static,
        I: Send + 'static,
    {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, Arc::new(CoreCancellation { handle }));
    }

    fn remove(&self, id: &str) {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    fn cancel(&self, id: &str) -> bool {
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned();
        if let Some(entry) = entry {
            entry.cancel();
            true
        } else {
            false
        }
    }
}

pub(crate) struct OperationRegistration<R: Runtime> {
    app: AppHandle<R>,
    id: String,
}

/// 把一个长操作登记进取消表，并返回一枚在作用域结束时自动注销的凭据。
///
/// 抽出来是为了让语音命令与赏析命令走**同一条**登记路径：两处各写一遍
/// 「insert 之后别忘了建 `OperationRegistration`」，忘掉的那一处会留下一个永远无法被
/// 取消的僵尸条目，而那种泄漏不会报错。
pub(crate) fn register_operation<R, P, I>(
    app: &AppHandle<R>,
    id: String,
    handle: Arc<OperationHandle<P, I>>,
) -> OperationRegistration<R>
where
    R: Runtime,
    P: Send + 'static,
    I: Send + 'static,
{
    app.state::<AppState>()
        .operations
        .insert(id.clone(), handle);
    OperationRegistration {
        app: app.clone(),
        id,
    }
}

impl<R: Runtime> Drop for OperationRegistration<R> {
    fn drop(&mut self) {
        self.app.state::<AppState>().operations.remove(&self.id);
    }
}

#[derive(Clone)]
struct AudioAsset {
    bytes: Arc<[u8]>,
    mime: String,
}

#[derive(Default)]
struct AudioStore {
    next: AtomicU64,
    assets: Mutex<HashMap<String, AudioAsset>>,
}

impl AudioStore {
    fn put(&self, bytes: Vec<u8>, mime: impl Into<String>) -> String {
        let sequence = self.next.fetch_add(1, Ordering::Relaxed);
        let token = format!("{sequence:016x}");
        self.assets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                token.clone(),
                AudioAsset {
                    bytes: bytes.into(),
                    mime: mime.into(),
                },
            );
        format!("{AUDIO_SCHEME}://localhost/{token}")
    }

    fn take(&self, token: &str) -> Option<AudioAsset> {
        self.assets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(token)
    }
}

pub(crate) fn configure_builder<R: Runtime>(builder: Builder<R>, config: Config) -> Builder<R> {
    builder
        .manage(AppState::new(config))
        .invoke_handler(tauri::generate_handler![
            search_text,
            browse_by_tag,
            list_tags,
            poem_detail,
            poem_annotations,
            lookup_dictionary,
            appreciate_poem,
            cancel_operation,
            key_status,
            set_api_key,
            delete_api_key,
            read_ai_settings,
            write_ai_settings,
            corpus_status,
            fetch_corpus,
            list_models,
            cache_status,
            purge_cache,
            recite_start_session,
            recite_submit_answer,
            recite_commit_grade,
            recite_due,
            recite_stats,
            crate::updater::update_check,
            crate::updater::update_download_and_install,
            crate::voice_ipc::voice_availability,
            crate::voice_ipc::voice_demonstrate,
            crate::voice_ipc::voice_start_session,
            crate::voice_ipc::voice_fetch_model,
        ])
        .register_uri_scheme_protocol(AUDIO_SCHEME, |context, request| {
            audio_response(context.app_handle(), request)
        })
}

fn audio_response<R: Runtime>(
    app: &AppHandle<R>,
    request: HttpRequest<Vec<u8>>,
) -> HttpResponse<Vec<u8>> {
    let token = request.uri().path().trim_start_matches('/');
    let Some(asset) = app.state::<AppState>().audio.take(token) else {
        return HttpResponse::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(b"audio token not found".to_vec())
            .expect("静态 404 响应合法");
    };
    HttpResponse::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, asset.mime)
        .header(header::CACHE_CONTROL, "no-store")
        .body(asset.bytes.as_ref().to_vec())
        .expect("音频响应合法")
}

pub(crate) async fn blocking<R, T, F>(app: AppHandle<R>, work: F) -> IpcResult<T>
where
    R: Runtime,
    T: Send + 'static,
    F: FnOnce(&AppState) -> IpcResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        work(&state)
    })
    .await
    .map_err(|error| format!("后台任务异常退出：{error}"))?
}

fn open_client(state: &AppState) -> IpcResult<Yunjian> {
    let config = state.config();
    CorpusHandle::open(&config.corpus)
        .map(Yunjian::new)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn search_text<R: Runtime>(
    app: AppHandle<R>,
    request: TextSearchRequest,
) -> IpcResult<SearchPage> {
    blocking(app, move |state| {
        open_client(state)?
            .search_text(request)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn browse_by_tag<R: Runtime>(
    app: AppHandle<R>,
    request: TagBrowseRequest,
) -> IpcResult<MetaPage> {
    blocking(app, move |state| {
        open_client(state)?
            .browse_by_tag(request)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn list_tags<R: Runtime>(app: AppHandle<R>) -> IpcResult<Vec<TagSummary>> {
    blocking(app, move |state| {
        open_client(state)?
            .list_tags()
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn poem_detail<R: Runtime>(
    app: AppHandle<R>,
    request: PoemDetailRequest,
) -> IpcResult<PoemDetail> {
    blocking(app, move |state| {
        open_client(state)?
            .poem_detail(request)
            .map_err(|error| error.to_string())
    })
    .await
}

/// 整首注音的请求。
///
/// **正文由调用方带下来，而不是这里再查一次语料库。** 详情页已经拿着正文了，让这个命令
/// 自己去查会把一次批量预取变成两次查询，而它真正要做的只是纯 CPU 的解析。这样一来
/// 「注音不碰数据库」在这条命令上也是结构性的。
#[derive(Debug, Deserialize)]
struct PoemAnnotationRequest {
    poem_id: String,
    body: String,
}

/// 整首注音的结果。
#[derive(Debug, Serialize)]
struct PoemAnnotationOut {
    /// 原样回带，供界面确认这份注音属于它当前展示的那首。
    poem_id: String,
    #[serde(flatten)]
    annotation: Annotation,
}

/// 随包破读词表只解析一次。
///
/// 词表是 `include_str!` 进来的常量，解析结果在整个进程生命周期里不变；每次调用重解析
/// 一遍没有正确性收益。解析失败要留住原因而不是退化成空表——空表会让三个黄金破读悄悄
/// 变成「多候选存疑」，那是最难发现的一种坏法。
static SHIPPED_POYIN: std::sync::OnceLock<Result<Poyin, String>> = std::sync::OnceLock::new();

fn shipped_poyin() -> Result<&'static Poyin, String> {
    SHIPPED_POYIN
        .get_or_init(|| Poyin::shipped().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(Clone::clone)
}

#[tauri::command]
async fn poem_annotations<R: Runtime>(
    app: AppHandle<R>,
    request: PoemAnnotationRequest,
) -> IpcResult<PoemAnnotationOut> {
    blocking(app, move |_state| {
        let poyin = shipped_poyin()?;
        Ok(PoemAnnotationOut {
            annotation: annotate_poem(poyin, &request.body),
            poem_id: request.poem_id,
        })
    })
    .await
}

#[tauri::command]
async fn lookup_dictionary<R: Runtime>(
    app: AppHandle<R>,
    request: DictionaryLookupRequest,
) -> IpcResult<DictionaryLookup> {
    blocking(app, move |state| {
        open_client(state)?
            .lookup_dictionary(request)
            .map_err(|error| error.to_string())
    })
    .await
}

#[derive(Debug, Deserialize)]
struct AppreciationCommandRequest {
    poem_id: String,
    #[serde(default)]
    operation_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AppreciationState {
    Ready { view: AppreciationView },
    Absent,
    ConfigurationRequired { settings_path: String },
    Failed { message: String },
}

#[derive(Debug, Serialize)]
struct AppreciationView {
    text: String,
    model: String,
    template_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'static str>,
}

impl AppreciationView {
    fn from_appreciation(appreciation: Appreciation, source: Option<CacheSource>) -> Self {
        Self {
            text: appreciation.text,
            model: appreciation.model,
            template_version: appreciation.template_version,
            source: source.map(cache_source_key),
        }
    }
}

fn cache_source_key(source: CacheSource) -> &'static str {
    match source {
        CacheSource::Shipped => "shipped",
        CacheSource::Local => "cache",
        CacheSource::Generated => "generated",
    }
}

enum PreparedAppreciation {
    Hit(CacheHit),
    Generate {
        provider: GenAiProvider,
        request: Box<AppreciationRequest>,
        app_data_dir: PathBuf,
        corpus_version: String,
    },
    Absent,
    ConfigurationRequired(String),
}

fn prepare_appreciation(state: &AppState, poem_id: String) -> IpcResult<PreparedAppreciation> {
    let config = state.config();
    let corpus = CorpusHandle::open(&config.corpus).map_err(|error| error.to_string())?;
    let detail = Yunjian::new(corpus.clone())
        .poem_detail(PoemDetailRequest { poem_id })
        .map_err(|error| error.to_string())?;
    let model = config.ai.model.clone().unwrap_or_else(|| {
        ProviderKind::parse(&config.ai.provider).map_or_else(
            |_| "shipped".to_owned(),
            |kind| kind.default_model().to_owned(),
        )
    });
    let request = AppreciationRequest::new(detail, model);
    let cache = AppreciationCache::open(
        &config.app.data_dir,
        corpus.meta().corpus_version.clone(),
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )
    .map_err(|error| error.to_string())?;
    let lookup_provider =
        ProviderId::new(&config.ai.provider).map_err(|error| error.to_string())?;
    if let Some(hit) = cache
        .lookup(&request, &lookup_provider)
        .map_err(|error| error.to_string())?
    {
        return Ok(PreparedAppreciation::Hit(hit));
    }
    if config.ai.provider == yunjian_core::config::PROVIDER_NONE {
        return Ok(PreparedAppreciation::Absent);
    }
    let kind = ProviderKind::parse(&config.ai.provider).map_err(|error| error.to_string())?;
    let mut provider_config = GenAiProviderConfig::new(kind);
    if let Some(endpoint) = config.ai.endpoint {
        provider_config = provider_config.with_base_url(endpoint);
    }
    if let Some(model) = config.ai.model {
        provider_config = provider_config.with_model_override(model);
    }
    let provider = match GenAiProvider::from_keystore(provider_config, &state.key_store) {
        Ok(provider) => provider,
        Err(yunjian_core::Error::AiKeyNotConfigured { .. }) => {
            return Ok(PreparedAppreciation::ConfigurationRequired(
                state.config_path.as_deref().map_or_else(
                    || "config.toml".to_owned(),
                    |path| path.display().to_string(),
                ),
            ));
        }
        Err(error) => return Err(error.to_string()),
    };
    Ok(PreparedAppreciation::Generate {
        provider,
        request: Box::new(request),
        app_data_dir: config.app.data_dir,
        corpus_version: corpus.meta().corpus_version.clone(),
    })
}

#[tauri::command]
async fn appreciate_poem<R: Runtime>(
    app: AppHandle<R>,
    request: AppreciationCommandRequest,
    on_event: Channel<Event<AppreciationProgress, AppreciationStreamItem>>,
) -> IpcResult<AppreciationState> {
    let operation_id = request.operation_id.unwrap_or_else(new_operation_id);
    let prepared = blocking(app.clone(), move |state| {
        prepare_appreciation(state, request.poem_id)
    })
    .await?;
    let PreparedAppreciation::Generate {
        provider,
        request,
        app_data_dir,
        corpus_version,
    } = prepared
    else {
        return Ok(match prepared {
            PreparedAppreciation::Hit(hit) => AppreciationState::Ready {
                view: AppreciationView::from_appreciation(hit.appreciation, Some(hit.source)),
            },
            PreparedAppreciation::Absent => AppreciationState::Absent,
            PreparedAppreciation::ConfigurationRequired(settings_path) => {
                AppreciationState::ConfigurationRequired { settings_path }
            }
            PreparedAppreciation::Generate { .. } => unreachable!(),
        });
    };

    let handle = provider
        .appreciate_stream((*request).clone())
        .await
        .map_err(|error| error.to_string())?;
    let handle = Arc::new(handle);
    let _registration = register_operation(&app, operation_id, Arc::clone(&handle));

    let mut completed = None;
    loop {
        if let Some(event) = next_event(&handle, 0) {
            if let Event::Item(AppreciationStreamItem::Complete(appreciation)) = &event {
                completed = Some(appreciation.clone());
            }
            if let Err(error) = on_event.send(event.clone()) {
                cancel(&handle);
                return Err(format!("发送赏析流失败：{error}"));
            }
            match event {
                Event::Done => break,
                Event::Cancelled => return Err("赏析已取消".to_owned()),
                Event::Failed { message } => return Ok(AppreciationState::Failed { message }),
                Event::Progress(_) | Event::Item(_) => {}
            }
        } else {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    let Some(appreciation) = completed else {
        return Ok(AppreciationState::Failed {
            message: "赏析流结束但没有完整结果".to_owned(),
        });
    };
    let cache_request = *request;
    let cache_appreciation = appreciation.clone();
    blocking(app, move |_| {
        let cache = AppreciationCache::open(
            app_data_dir,
            corpus_version,
            DEFAULT_APPRECIATION_CACHE_CAPACITY,
        )
        .map_err(|error| error.to_string())?;
        cache
            .store_completed(&cache_request, &cache_appreciation)
            .map_err(|error| error.to_string())
    })
    .await?;
    Ok(AppreciationState::Ready {
        view: AppreciationView::from_appreciation(appreciation, Some(CacheSource::Generated)),
    })
}

pub(crate) fn new_operation_id() -> String {
    static NEXT_OPERATION: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("op-{nanos:x}-{sequence:x}")
}

#[tauri::command]
pub(crate) fn cancel_operation<R: Runtime>(app: AppHandle<R>, operation_id: String) -> bool {
    app.state::<AppState>().operations.cancel(&operation_id)
}

#[derive(Debug, Serialize)]
struct KeyStatus {
    report: yunjian_ai::StorageReport,
    needs_reprompt: bool,
}

#[tauri::command]
async fn key_status<R: Runtime>(app: AppHandle<R>, provider: String) -> IpcResult<KeyStatus> {
    blocking(app, move |state| {
        let lookup = state
            .key_store
            .get(&provider)
            .map_err(|error| error.to_string())?;
        Ok(KeyStatus {
            report: lookup.report().clone(),
            needs_reprompt: lookup.needs_reprompt(),
        })
    })
    .await
}

#[tauri::command]
async fn set_api_key<R: Runtime>(
    app: AppHandle<R>,
    provider: String,
    secret: String,
) -> IpcResult<yunjian_ai::StorageReport> {
    blocking(app, move |state| {
        state
            .key_store
            .set(&provider, &SecretString::from(secret))
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn delete_api_key<R: Runtime>(
    app: AppHandle<R>,
    provider: String,
) -> IpcResult<yunjian_ai::StorageReport> {
    blocking(app, move |state| {
        state
            .key_store
            .delete(&provider)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn read_ai_settings<R: Runtime>(app: AppHandle<R>) -> IpcResult<AiConfig> {
    blocking(app, move |state| Ok(state.config().ai)).await
}

#[tauri::command]
async fn write_ai_settings<R: Runtime>(app: AppHandle<R>, settings: AiConfig) -> IpcResult<()> {
    blocking(app, move |state| {
        let mut config = state.config();
        config.ai = settings;
        let path = state
            .config_path
            .as_deref()
            .ok_or_else(|| "无法解析用户配置目录".to_owned())?;
        atomic_write_config(path, &config)?;
        state.replace_config(config);
        Ok(())
    })
    .await
}

fn atomic_write_config(path: &Path, config: &Config) -> IpcResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败：{error}"))?;
    }
    let source =
        toml::to_string_pretty(config).map_err(|error| format!("序列化配置失败：{error}"))?;
    let temp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    let mut file = fs::File::create(&temp).map_err(|error| format!("创建临时配置失败：{error}"))?;
    file.write_all(source.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("写入临时配置失败：{error}"))?;
    fs::rename(&temp, path).map_err(|error| format!("原子替换配置失败：{error}"))
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CorpusStatus {
    Ready { meta: CorpusMetaOut },
    Absent,
}

#[derive(Debug, Serialize)]
struct CorpusMetaOut {
    schema_version: u32,
    corpus_version: String,
    built_at: String,
    poem_count: i64,
    index_detail_mode: String,
    derived_indexes: String,
    shipped_scope: String,
}

fn corpus_status_for(config: &Config) -> IpcResult<CorpusStatus> {
    let candidate = config
        .corpus
        .path
        .clone()
        .unwrap_or_else(|| config.corpus.data_dir.join(yunjian_core::CORPUS_FILE_NAME));
    if !candidate.is_file() {
        return Ok(CorpusStatus::Absent);
    }
    let mut corpus_config = config.corpus.clone();
    corpus_config.path = Some(candidate);
    let corpus = CorpusHandle::open(&corpus_config).map_err(|error| error.to_string())?;
    let meta = corpus.meta();
    Ok(CorpusStatus::Ready {
        meta: CorpusMetaOut {
            schema_version: meta.schema_version,
            corpus_version: meta.corpus_version.clone(),
            built_at: meta.built_at.clone(),
            poem_count: meta.poem_count,
            index_detail_mode: meta.index_detail_mode.clone(),
            derived_indexes: meta.derived_indexes.clone(),
            shipped_scope: meta.shipped_scope.clone(),
        },
    })
}

#[tauri::command]
async fn corpus_status<R: Runtime>(app: AppHandle<R>) -> IpcResult<CorpusStatus> {
    blocking(app, move |state| corpus_status_for(&state.config())).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
enum CorpusProgress {
    AlreadyPresent {
        path: String,
    },
    VerifyingArchive {
        archive: String,
        bytes: u64,
    },
    ArchiveVerified {
        sha256: String,
    },
    Decompressing {
        bytes_done: u64,
        bytes_total: u64,
    },
    Materialized {
        path: String,
        corpus_version: String,
    },
    Deriving {
        detail: String,
    },
    DeriveFailed {
        reason: String,
    },
    Ready {
        path: String,
        corpus_version: String,
        derived: bool,
    },
}

impl From<MaterializationProgress<'_>> for CorpusProgress {
    fn from(progress: MaterializationProgress<'_>) -> Self {
        match progress {
            MaterializationProgress::AlreadyPresent { path } => Self::AlreadyPresent {
                path: path.display().to_string(),
            },
            MaterializationProgress::VerifyingArchive { archive, bytes } => {
                Self::VerifyingArchive {
                    archive: archive.display().to_string(),
                    bytes,
                }
            }
            MaterializationProgress::ArchiveVerified { sha256 } => Self::ArchiveVerified {
                sha256: sha256.to_owned(),
            },
            MaterializationProgress::Decompressing {
                bytes_done,
                bytes_total,
            } => Self::Decompressing {
                bytes_done,
                bytes_total,
            },
            MaterializationProgress::Materialized {
                path,
                corpus_version,
            } => Self::Materialized {
                path: path.display().to_string(),
                corpus_version: corpus_version.to_owned(),
            },
            MaterializationProgress::Deriving(progress) => Self::Deriving {
                detail: format!("{progress:?}"),
            },
            MaterializationProgress::DeriveFailed { reason } => Self::DeriveFailed {
                reason: reason.to_owned(),
            },
            MaterializationProgress::Ready {
                path,
                corpus_version,
                derived,
            } => Self::Ready {
                path: path.display().to_string(),
                corpus_version: corpus_version.to_owned(),
                derived,
            },
        }
    }
}

#[tauri::command]
async fn fetch_corpus<R: Runtime>(
    app: AppHandle<R>,
    on_event: Channel<Event<CorpusProgress, Value>>,
) -> IpcResult<CorpusStatus> {
    blocking(app, move |state| {
        let config = state.config();
        let result = CorpusHandle::open_with_progress(&config.corpus, &mut |progress| {
            let _ = on_event.send(Event::Progress(CorpusProgress::from(progress)));
        });
        match result {
            Ok(corpus) => {
                let _ = on_event.send(Event::Done);
                let meta = corpus.meta();
                Ok(CorpusStatus::Ready {
                    meta: CorpusMetaOut {
                        schema_version: meta.schema_version,
                        corpus_version: meta.corpus_version.clone(),
                        built_at: meta.built_at.clone(),
                        poem_count: meta.poem_count,
                        index_detail_mode: meta.index_detail_mode.clone(),
                        derived_indexes: meta.derived_indexes.clone(),
                        shipped_scope: meta.shipped_scope.clone(),
                    },
                })
            }
            Err(error) => {
                let _ = on_event.send(Event::Failed {
                    message: error.to_string(),
                });
                Err(error.to_string())
            }
        }
    })
    .await
}

#[tauri::command]
async fn list_models<R: Runtime>(app: AppHandle<R>) -> IpcResult<Value> {
    blocking(app, move |state| {
        let statuses = ModelCache::at(state.config().voice.model_dir)
            .statuses()
            .map_err(|error| error.to_string())?;
        Ok(Value::Array(
            statuses
                .into_iter()
                .map(|status| {
                    json!({
                        "name": status.name,
                        "kind": status.kind.as_str(),
                        "role": status.role.as_str(),
                        "license": status.license,
                        "size_bytes": status.size_bytes,
                        "unpacked": status.unpacked,
                        "archived": status.archived,
                        "refused": status.refused,
                        "attribution": status.attribution,
                    })
                })
                .collect(),
        ))
    })
    .await
}

fn open_cache(config: &Config) -> IpcResult<AppreciationCache> {
    let corpus_version = corpus_status_for(config).and_then(|status| match status {
        CorpusStatus::Ready { meta } => Ok(meta.corpus_version),
        CorpusStatus::Absent => Err("语料尚未就绪".to_owned()),
    })?;
    AppreciationCache::open(
        &config.app.data_dir,
        corpus_version,
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn cache_status<R: Runtime>(app: AppHandle<R>) -> IpcResult<Value> {
    blocking(app, move |state| {
        let config = state.config();
        let cache = open_cache(&config)?;
        let counts = cache.counts().map_err(|error| error.to_string())?;
        let database_bytes = fs::metadata(cache.path())
            .ok()
            .map(|metadata| metadata.len());
        Ok(json!({
            "counts": { "shipped": counts.shipped, "local": counts.local },
            "database_bytes": database_bytes,
        }))
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PurgeScopeIn {
    Template { template_version: String },
    Poem { poem_id: String },
    All,
}

#[tauri::command]
async fn purge_cache<R: Runtime>(app: AppHandle<R>, scope: PurgeScopeIn) -> IpcResult<usize> {
    blocking(app, move |state| {
        let scope = match scope {
            PurgeScopeIn::Template { template_version } => PurgeScope::Template(template_version),
            PurgeScopeIn::Poem { poem_id } => PurgeScope::Poem(poem_id),
            PurgeScopeIn::All => PurgeScope::All,
        };
        open_cache(&state.config())?
            .purge(scope)
            .map_err(|error| error.to_string())
    })
    .await
}

#[derive(Debug, Clone, Deserialize)]
struct ReciteSessionRequest {
    poem_id: String,
    mode: String,
    #[serde(default)]
    ratio: Option<f32>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    masked_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReciteAnswerRequest {
    poem_id: String,
    mode: String,
    #[serde(default)]
    ratio: Option<f32>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    masked_lines: Option<usize>,
    answer: String,
}

impl From<ReciteAnswerRequest> for (ReciteSessionRequest, String) {
    fn from(request: ReciteAnswerRequest) -> Self {
        (
            ReciteSessionRequest {
                poem_id: request.poem_id,
                mode: request.mode,
                ratio: request.ratio,
                seed: request.seed,
                masked_lines: request.masked_lines,
            },
            request.answer,
        )
    }
}

fn effective_seed(request: &ReciteSessionRequest) -> u64 {
    request.seed.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64)
    })
}

struct BuiltSession {
    request: ReciteSessionRequest,
    detail: PoemDetail,
    session: PracticeSession,
    requested_mode: Option<&'static str>,
    fallback_reason: Option<String>,
}

fn build_session(state: &AppState, mut request: ReciteSessionRequest) -> IpcResult<BuiltSession> {
    let config = state.config();
    let corpus = CorpusHandle::open(&config.corpus).map_err(|error| error.to_string())?;
    let detail = Yunjian::new(corpus.clone())
        .poem_detail(PoemDetailRequest {
            poem_id: request.poem_id.clone(),
        })
        .map_err(|error| error.to_string())?;
    let seed = effective_seed(&request);
    request.seed = Some(seed);
    let (mode, requested_mode, fallback_reason) = match request.mode.as_str() {
        "cloze" => (
            PracticeMode::Cloze(ClozeOptions::new(
                request.ratio.unwrap_or(ClozeOptions::DEFAULT_RATIO),
                seed,
            )),
            None,
            None,
        ),
        "first-char" => (PracticeMode::FirstChar, None, None),
        "masked" => (
            PracticeMode::Masked(MaskStage::new(request.masked_lines.unwrap_or(0))),
            None,
            None,
        ),
        // 语音跟读有自己的端点（`voice_start_session`），因为它的产出与打字评分刻意不可
        // 互换。打字端点收到 `voice` 时仍然给一局挖空，但**原因取自语音装置的实测判定**
        // 而不是一句写死的「尚未接入」——那句话在语音接进来之后就是假的，而假的降级原因
        // 会把用户指去检查一个并不存在的问题。
        "voice" => (
            PracticeMode::Cloze(ClozeOptions::new(
                request.ratio.unwrap_or(ClozeOptions::DEFAULT_RATIO),
                seed,
            )),
            Some("voice"),
            Some(match state.voice_rig().probe(&config) {
                Practice::Voice => "语音跟读走独立端点；打字端点已给出一局挖空练习".to_owned(),
                Practice::Typed { message, .. } => message,
            }),
        ),
        other => return Err(format!("未知背诵形态：{other}")),
    };
    let session = PracticeSession::start(&corpus, &detail.poem.body, mode)
        .map_err(|error| error.to_string())?;
    Ok(BuiltSession {
        request,
        detail,
        session,
        requested_mode,
        fallback_reason,
    })
}

fn mode_fields(mode: PracticeMode) -> (&'static str, Option<f32>, Option<u64>, Option<usize>) {
    match mode {
        PracticeMode::Cloze(options) => {
            ("cloze", Some(options.ratio()), Some(options.seed()), None)
        }
        PracticeMode::FirstChar => ("first-char", None, None, None),
        PracticeMode::Masked(stage) => ("masked", None, None, Some(stage.masked_lines())),
    }
}

fn session_json(built: &BuiltSession) -> Value {
    let (mode, ratio, seed, masked_lines) = mode_fields(built.session.mode());
    json!({
        "poem_id": built.request.poem_id,
        "title": built.detail.poem.title,
        "author": built.detail.poem.author,
        "dynasty": built.detail.poem.dynasty.raw,
        "mode": mode,
        "requested_mode": built.requested_mode,
        "fallback_reason": built.fallback_reason,
        "ratio": ratio,
        "seed": seed,
        "masked_lines": masked_lines,
        "prompt": built.session.prompt(),
        "hidden_indices": built.session.hidden_indices(),
        "line_count": built.session.line_count(),
    })
}

#[tauri::command]
async fn recite_start_session<R: Runtime>(
    app: AppHandle<R>,
    request: ReciteSessionRequest,
) -> IpcResult<Value> {
    blocking(app, move |state| {
        build_session(state, request).map(|built| session_json(&built))
    })
    .await
}

#[tauri::command]
async fn recite_submit_answer<R: Runtime>(
    app: AppHandle<R>,
    request: ReciteAnswerRequest,
) -> IpcResult<Value> {
    blocking(app, move |state| {
        let (session_request, answer) = request.into();
        let built = build_session(state, session_request)?;
        let config = state.config();
        let corpus = CorpusHandle::open(&config.corpus).map_err(|error| error.to_string())?;
        let (normalized_answer, review) =
            review_typed_text(&corpus, built.session.reference(), &answer)
                .map_err(|error| error.to_string())?;
        let alignment =
            align(&corpus, &built.detail.poem.body, &answer).map_err(|error| error.to_string())?;
        let database = config.app.data_dir.join(RECITE_DATABASE_FILE);
        let scheduler = Scheduler::open(&database).map_err(|error| error.to_string())?;
        let first_attempt = scheduler
            .state(&built.request.poem_id)
            .map_err(|error| error.to_string())?
            .is_none();
        let suggested = grade_typed(&review.score, first_attempt, &config.recite.grading);
        let mut value = session_json(&built);
        let object = value.as_object_mut().expect("session_json 恒为对象");
        object.insert(
            "reference".to_owned(),
            json!(built.session.reference().as_str()),
        );
        object.insert("answer".to_owned(), json!(normalized_answer));
        object.insert("score".to_owned(), score_json(&review.score));
        object.insert(
            "ops".to_owned(),
            Value::Array(alignment.ops.iter().map(op_json).collect()),
        );
        object.insert("suggested_grade".to_owned(), json!(grade_key(suggested)));
        object.insert("first_attempt".to_owned(), json!(first_attempt));
        object.insert("database".to_owned(), json!(database.display().to_string()));
        Ok(value)
    })
    .await
}

#[derive(Debug, Deserialize)]
struct ReciteCommitRequest {
    poem_id: String,
    grade: String,
    chosen_by_user: bool,
}

#[tauri::command]
async fn recite_commit_grade<R: Runtime>(
    app: AppHandle<R>,
    request: ReciteCommitRequest,
) -> IpcResult<Value> {
    blocking(app, move |state| {
        let config = state.config();
        let database = config.app.data_dir.join(RECITE_DATABASE_FILE);
        let grade = parse_grade(&request.grade)?;
        let review = Scheduler::open(&database)
            .map_err(|error| error.to_string())?
            .review(&request.poem_id, grade)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "grade": grade_key(grade),
            "grade_source": if request.chosen_by_user { "user_chosen" } else { "typed_mapping" },
            "database": database.display().to_string(),
            "review": review_json(&review),
        }))
    })
    .await
}

#[tauri::command]
async fn recite_due<R: Runtime>(app: AppHandle<R>, include_future: bool) -> IpcResult<Value> {
    blocking(app, move |state| {
        let config = state.config();
        let database = config.app.data_dir.join(RECITE_DATABASE_FILE);
        let scheduler = Scheduler::open(&database).map_err(|error| error.to_string())?;
        let states = if include_future {
            scheduler.due_on(i64::MAX)
        } else {
            scheduler.due_today()
        }
        .map_err(|error| error.to_string())?;
        Ok(json!({
            "database": database.display().to_string(),
            "scope": if include_future { "all" } else { "due_today" },
            "items": states.iter().map(review_json).collect::<Vec<_>>(),
        }))
    })
    .await
}

#[tauri::command]
async fn recite_stats<R: Runtime>(app: AppHandle<R>) -> IpcResult<Value> {
    blocking(app, move |state| {
        let config = state.config();
        let database = config.app.data_dir.join(RECITE_DATABASE_FILE);
        let scheduler = Scheduler::open(&database).map_err(|error| error.to_string())?;
        let scheduled = scheduler
            .due_on(i64::MAX)
            .map_err(|error| error.to_string())?;
        let due_today = scheduler
            .due_today()
            .map_err(|error| error.to_string())?
            .len();
        let mut grades = json!({ "again": 0, "hard": 0, "good": 0, "easy": 0 });
        for state in &scheduled {
            let key = grade_key(state.last_grade);
            let count = grades[key].as_u64().unwrap_or_default();
            grades[key] = json!(count + 1);
        }
        Ok(json!({
            "database": database.display().to_string(),
            "scheduled_total": scheduled.len(),
            "due_today": due_today,
            "by_last_grade": grades,
            "grading": config.recite.grading,
        }))
    })
    .await
}

fn score_json(score: &TypedScore) -> Value {
    json!({
        "completeness": score.completeness,
        "accuracy_strict": score.accuracy_strict,
        "accuracy_lenient": score.accuracy_lenient,
        "fluency": score.fluency,
        "is_rejected": score.is_rejected,
        "ops_summary": {
            "normal_count": score.ops_summary.normal_count,
            "deletion_count": score.ops_summary.deletion_count,
            "insertion_count": score.ops_summary.insertion_count,
            "rerecitation_count": score.ops_summary.rerecitation_count,
            "substitution_count": score.ops_summary.substitution_count,
        }
    })
}

fn op_json(op: &AlignOp) -> Value {
    match op {
        AlignOp::Normal {
            reference_index,
            attempt_index,
            character,
        } => json!({
            "kind": "normal", "reference_index": reference_index,
            "attempt_index": attempt_index, "character": character,
        }),
        AlignOp::Deletion {
            reference_index,
            reference,
        } => json!({
            "kind": "deletion", "reference_index": reference_index, "reference": reference,
        }),
        AlignOp::Insertion {
            reference_index,
            attempt_index,
            attempt,
        } => json!({
            "kind": "insertion", "reference_index": reference_index,
            "attempt_index": attempt_index, "attempt": attempt,
        }),
        AlignOp::ReRecitation {
            reference_start,
            reference_end,
            attempt_start,
            attempt_end,
            text,
        } => json!({
            "kind": "re_recitation", "reference_start": reference_start,
            "reference_end": reference_end, "attempt_start": attempt_start,
            "attempt_end": attempt_end, "text": text,
        }),
        AlignOp::Substitution {
            reference_index,
            attempt_index,
            reference,
            attempt,
        } => json!({
            "kind": "substitution", "reference_index": reference_index,
            "attempt_index": attempt_index, "reference": reference, "attempt": attempt,
            "near_homophone": classify_substitution(*reference, *attempt)
                == SubstitutionClass::NearHomophone,
        }),
    }
}

fn review_json(review: &ReviewState) -> Value {
    json!({
        "poem_id": review.stable_id,
        "due_day": review.due_day,
        "last_review_day": review.last_review_day,
        "scheduled_days": review.scheduled_days,
        "stability": review.stability,
        "difficulty": review.difficulty,
        "last_grade": grade_key(review.last_grade),
    })
}

fn grade_key(grade: FsrsGrade) -> &'static str {
    match grade {
        FsrsGrade::Again => "again",
        FsrsGrade::Hard => "hard",
        FsrsGrade::Good => "good",
        FsrsGrade::Easy => "easy",
    }
}

fn parse_grade(value: &str) -> IpcResult<FsrsGrade> {
    match value {
        "again" => Ok(FsrsGrade::Again),
        "hard" => Ok(FsrsGrade::Hard),
        "good" => Ok(FsrsGrade::Good),
        "easy" => Ok(FsrsGrade::Easy),
        other => Err(format!("未知复习等级：{other}")),
    }
}

#[cfg(test)]
mod testing {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use tauri::ipc::{Channel, InvokeResponseBody};
    use yunjian_core::operation::testing::{ConformanceAdapter, assert_conforms};
    use yunjian_core::operation::{
        Event, OperationHandle, OperationReporter, cancel, close, next_event, start_operation,
    };

    struct ChannelHandle {
        operation: OperationHandle<u16, u16>,
        channel: Channel<Event<u16, u16>>,
        delivered: Arc<Mutex<Vec<Event<u16, u16>>>>,
    }

    #[derive(Clone, Copy)]
    struct ChannelAdapter;

    impl ConformanceAdapter for ChannelAdapter {
        type Handle = ChannelHandle;

        fn start<F>(&self, producer: F) -> Self::Handle
        where
            F: FnOnce(OperationReporter<u16, u16>) -> Result<(), String> + Send + 'static,
        {
            let delivered = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&delivered);
            let channel = Channel::new(move |body| {
                let InvokeResponseBody::Json(source) = body else {
                    panic!("期望 JSON Channel 载荷");
                };
                let event = serde_json::from_str(&source)?;
                sink.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(event);
                Ok(())
            });
            ChannelHandle {
                operation: start_operation(producer),
                channel,
                delivered,
            }
        }

        fn next_event(&self, handle: &Self::Handle, timeout_ms: u64) -> Option<Event<u16, u16>> {
            let event = next_event(&handle.operation, timeout_ms)?;
            handle
                .channel
                .send(event)
                .expect("Channel 序列化并投递事件");
            handle
                .delivered
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(0)
                .into()
        }

        fn cancel(&self, handle: &Self::Handle) {
            cancel(&handle.operation);
        }

        fn close(&self, handle: &Self::Handle) {
            close(&handle.operation);
        }
    }

    pub(super) fn assert_channel_adapter_conforms() {
        assert_conforms(&ChannelAdapter);
    }

    pub(super) fn assert_cancel_is_prompt() {
        let operation = Arc::new(start_operation(|reporter: OperationReporter<(), ()>| {
            while !reporter.wait_for_stop(Duration::from_millis(5)) {}
            Ok::<(), String>(())
        }));
        let registry = super::OperationRegistry::default();
        registry.insert("long".to_owned(), Arc::clone(&operation));
        let started = Instant::now();
        assert!(registry.cancel("long"));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(next_event(&operation, 500), Some(Event::Cancelled));
    }

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    pub(super) fn assert_appreciation_drop_is_prompt_and_uncached() {
        let dropped = Arc::new(AtomicBool::new(false));
        let cached = Arc::new(AtomicBool::new(false));
        let drop_probe = Arc::clone(&dropped);
        let cache_probe = Arc::clone(&cached);
        let operation = start_operation(move |reporter: OperationReporter<(), ()>| {
            let _future_guard = DropProbe(drop_probe);
            while !reporter.wait_for_stop(Duration::from_millis(2)) {}
            if !reporter.is_cancelled() && !reporter.is_closed() {
                cache_probe.store(true, Ordering::Release);
            }
            Ok::<(), String>(())
        });
        let started = Instant::now();
        drop(operation);
        while !dropped.load(Ordering::Acquire) && started.elapsed() < Duration::from_millis(500) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            dropped.load(Ordering::Acquire),
            "drop 必须在 500 ms 内传到网络 future"
        );
        assert!(
            !cached.load(Ordering::Acquire),
            "取消或失败不得写入赏析缓存"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    const ASYNC_COMMANDS: &[&str] = &[
        "search_text",
        "browse_by_tag",
        "list_tags",
        "poem_detail",
        "lookup_dictionary",
        "appreciate_poem",
        "key_status",
        "set_api_key",
        "delete_api_key",
        "read_ai_settings",
        "write_ai_settings",
        "corpus_status",
        "fetch_corpus",
        "list_models",
        "cache_status",
        "purge_cache",
        "recite_start_session",
        "recite_submit_answer",
        "recite_commit_grade",
        "recite_due",
        "recite_stats",
    ];

    fn source() -> String {
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ipc.rs"))
            .expect("读取 IPC 源码")
    }

    /// 生产源码，即测试模块之前的全部内容。
    ///
    /// 切分点是 `#[cfg(test)] mod` 这个**测试模块**标记，不是第一个 `#[cfg(test)]` 属性。
    /// 后者曾把一个 cfg 在测试下的生产辅助方法当成分界，于是它之后的真生产代码（包括
    /// 自定义 URI 协议的注册）整段落到切分点之外，让下面几条守卫在**看不见的一半源码上
    /// 恒真**。守卫悄悄缩小比守卫报错危险得多。
    fn production() -> String {
        let source = source();
        source
            .split("#[cfg(test)]\nmod ")
            .next()
            .expect("存在生产源码")
            .to_owned()
    }

    #[test]
    fn every_registered_business_command_is_async() {
        let source = source();
        for command in ASYNC_COMMANDS {
            assert!(
                source.contains(&format!("async fn {command}")),
                "IPC 命令 `{command}` 必须是 async，避免占用 WebView 主线程"
            );
        }
    }

    #[test]
    fn dictionary_database_lookup_runs_on_the_blocking_pool() {
        let production = production();
        let body = production
            .split("async fn lookup_dictionary")
            .nth(1)
            .and_then(|source| source.split("#[tauri::command]").next())
            .expect("存在字典 IPC 命令体");
        assert!(
            body.contains("blocking(app,"),
            "字典数据库查询必须进入 spawn_blocking 统一封装"
        );
    }

    #[test]
    fn dictionary_ipc_has_no_modern_lexicon_extension_surface() {
        let production = production();
        let without_comments = production
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "dictionary_provider",
            "dictionary_endpoint",
            "remote_dictionary",
            "online_dictionary",
        ] {
            assert!(
                !without_comments.contains(forbidden),
                "字典 IPC 不得预留现代辞书扩展口：{forbidden}"
            );
        }
    }

    #[test]
    fn blocking_workers_cannot_await_or_build_a_nested_runtime() {
        let source = source();
        for body in source.split("spawn_blocking(").skip(1) {
            let closure = body
                .split(".await")
                .next()
                .expect("spawn_blocking 后应存在闭包体");
            assert!(
                !closure.contains("await"),
                "spawn_blocking 闭包不得 await；网络流必须留在 async 命令中"
            );
            assert!(
                !closure.contains("Runtime::new") && !closure.contains("Builder::new_"),
                "spawn_blocking 闭包不得构造嵌套 async runtime"
            );
        }
    }

    #[test]
    fn streaming_uses_channels_without_events_or_eval() {
        let production = production();
        assert!(
            production.contains("ipc::Channel"),
            "流式数据必须通过 ipc::Channel"
        );
        assert!(
            !production.contains(".emit("),
            "流式路径不得使用 Tauri event"
        );
        assert!(!production.contains(".eval("), "不得通过 eval 传输数据");
    }

    #[test]
    fn no_command_returns_audio_as_a_byte_vector() {
        let production = production();
        assert!(
            !production.lines().any(|line| {
                line.contains("async fn")
                    && (line.contains("Vec<u8>") || line.contains("Vec < u8 >"))
            }),
            "命令不得返回 Vec<u8>；音频必须通过自定义 URI 协议读取"
        );
        assert!(
            production.contains("register_uri_scheme_protocol"),
            "桌面 builder 必须注册音频自定义 URI 协议"
        );
    }

    #[test]
    fn channel_adapter_obeys_the_workspace_operation_protocol() {
        super::testing::assert_channel_adapter_conforms();
    }

    #[test]
    fn synchronous_cancel_is_not_queued_behind_a_long_worker() {
        super::testing::assert_cancel_is_prompt();
    }

    #[test]
    fn dropping_appreciation_cancels_before_cache_write() {
        super::testing::assert_appreciation_drop_is_prompt_and_uncached();
    }
}
