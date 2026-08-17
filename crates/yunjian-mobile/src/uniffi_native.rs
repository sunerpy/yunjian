//! UniFFI 原生外壳适配器。
//!
//! 领域 crate 不派生任何 UniFFI 类型：短调用以 serde JSON 穿过边界，长调用则把
//! [`yunjian_core::operation`] 擦除成稳定的 JSON 事件对象。原生端可主动调用
//! [`NativeOperation::next_event`]，也可用 [`NativeEventSink`] 消费同一队列；取消与关闭仍
//! 原样转发核心协议。ASR 的 PCM 输入由 [`NativeAsrOperation`] 以有界队列送入现有 sherpa
//! 双路识别器，不复制识别逻辑，也不会把诊断假设送进评分。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "native-voice")]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "native-voice")]
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
#[cfg(feature = "native-voice")]
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use yunjian_ai::{
    AppreciationCache, AppreciationProvider, AppreciationRequest, CacheSource,
    DEFAULT_APPRECIATION_CACHE_CAPACITY, GenAiProvider, GenAiProviderConfig, KeyStore,
    KeyStoreConfig, NullProvider, ProviderId, ProviderKind,
};
#[cfg(feature = "native-voice")]
use yunjian_core::operation::start_operation;
use yunjian_core::operation::{Event, OperationHandle, cancel, close, next_event};
use yunjian_core::{CorpusConfig, CorpusHandle, PoemDetailRequest, TextSearchRequest};
use yunjian_recite::Scheduler;
#[cfg(feature = "native-voice")]
use yunjian_voice::asr::streaming::{StreamingDualDecoder, TransducerFiles};
#[cfg(feature = "native-voice")]
use yunjian_voice::recognize::{
    Hotwords, OnlineDecodeConfig, PartialHypothesis, PcmSource, Prompt, PromptReason,
    RecognitionItem, RecognitionOutcome, RecognitionPlan, RecognitionProgress, start_recognition,
};

use crate::{MobileFacade, ReciteStartRequest, ReciteSubmitRequest};

const CALLBACK_POLL_MS: u64 = 200;
#[cfg(feature = "native-voice")]
const ASR_FRAME_CAPACITY: usize = 32;

/// 可跨 UniFFI 边界抛出的脱敏错误。
#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum NativeError {
    /// 请求、配置或领域调用失败。
    Failure {
        /// 已脱敏、可直接展示给用户的错误信息。
        message: String,
    },
}

impl NativeError {
    fn message(message: impl Into<String>) -> Self {
        Self::Failure {
            message: message.into(),
        }
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failure { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for NativeError {}

fn native_error(error: impl fmt::Display) -> NativeError {
    NativeError::message(error.to_string())
}

fn decode_json<T: DeserializeOwned>(json: &str, label: &str) -> Result<T, NativeError> {
    serde_json::from_str(json)
        .map_err(|error| NativeError::message(format!("{label} JSON 无效：{error}")))
}

fn encode_json<T: Serialize>(value: &T) -> Result<String, NativeError> {
    serde_json::to_string(value).map_err(native_error)
}

trait ErasedOperation: Send + Sync {
    fn next_event_json(&self, timeout_ms: u64) -> Option<String>;
    fn cancel(&self);
    fn close(&self);
    fn is_finished(&self) -> bool;
}

struct TypedOperation<P, I> {
    handle: OperationHandle<P, I>,
    finished: AtomicBool,
}

impl<P, I> ErasedOperation for TypedOperation<P, I>
where
    P: Serialize + Send + 'static,
    I: Serialize + Send + 'static,
{
    fn next_event_json(&self, timeout_ms: u64) -> Option<String> {
        next_event(&self.handle, timeout_ms).map(|event| {
            if event.is_terminal() {
                self.finished.store(true, Ordering::Release);
            }
            serde_json::to_string(&event)
                .expect("核心 operation 事件由 Serialize 类型构成，序列化不应失败")
        })
    }

    fn cancel(&self) {
        cancel(&self.handle);
    }

    fn close(&self) {
        close(&self.handle);
        self.finished.store(true, Ordering::Release);
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

/// 原生端实现的事件接收器。
///
/// 回调可能从 Rust 工作线程调用；Kotlin/Swift 实现必须自行切回 UI 线程。
#[uniffi::export(callback_interface)]
pub trait NativeEventSink: Send + Sync {
    /// 接收一条与 Tauri Channel 逐字节同构的 JSON 事件。
    fn on_event(&self, event_json: String);
}

/// 跨 Kotlin/Swift 边界的长任务句柄。
#[derive(uniffi::Object)]
pub struct NativeOperation {
    inner: Box<dyn ErasedOperation>,
}

impl NativeOperation {
    /// 把任意核心 operation 擦除成原生边界对象。
    #[must_use]
    pub fn from_operation<P, I>(handle: OperationHandle<P, I>) -> Arc<Self>
    where
        P: Serialize + Send + 'static,
        I: Serialize + Send + 'static,
    {
        Arc::new(Self {
            inner: Box::new(TypedOperation {
                handle,
                finished: AtomicBool::new(false),
            }),
        })
    }
}

#[uniffi::export]
impl NativeOperation {
    /// 在超时内拉取下一事件；超时或终态已消费后返回 `None`。
    pub fn next_event(&self, timeout_ms: u64) -> Option<String> {
        self.inner.next_event_json(timeout_ms)
    }

    /// 在后台线程持续拉取事件并回调，直到收到唯一终态或句柄关闭。
    ///
    /// 一次轮询超时只代表生产者暂时没有新事件，不能结束订阅；慢网络模型因此不会在首个
    /// 200 ms 空窗后静默丢失后续 chunk。
    pub fn subscribe(self: Arc<Self>, sink: Box<dyn NativeEventSink>) {
        let _ = std::thread::Builder::new()
            .name("yunjian-uniffi-events".to_owned())
            .spawn(move || {
                loop {
                    match self.next_event(CALLBACK_POLL_MS) {
                        Some(event_json) => {
                            let terminal = serde_json::from_str::<
                                Event<serde_json::Value, serde_json::Value>,
                            >(&event_json)
                            .is_ok_and(|event| event.is_terminal());
                            sink.on_event(event_json);
                            if terminal {
                                break;
                            }
                        }
                        None if self.inner.is_finished() => break,
                        None => {}
                    }
                }
            });
    }

    /// 幂等地请求取消；Rust 生产者通过核心协议中的 flag 观察它。
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// 幂等地关闭句柄并释放尚未消费的事件。
    ///
    /// # 为什么不叫 `close`
    ///
    /// UniFFI 生成的 Kotlin 对象已经实现 `AutoCloseable`，其 `close()` 负责释放
    /// **Rust 侧的对象句柄**。再导出一个自己的 `close` 会在同一个类里产生两个
    /// `override fun close()`，Kotlin 编译器报 `Conflicting overloads`。
    ///
    /// 这个冲突只有在真正用 Kotlin 编译器编译生成物时才会暴露——`tests/architecture.rs`
    /// 做的是文本断言（`contains("open class NativeFacade")`），编译不到这一层。
    /// 本次把绑定接进 Gradle 才发现它。
    pub fn shutdown(&self) {
        self.inner.close();
    }
}

/// 构造原生共享门面的 JSON 配置。
#[derive(Debug, Deserialize)]
struct NativeFacadeConfig {
    corpus_path: Option<PathBuf>,
    corpus_data_dir: PathBuf,
    corpus_archive: Option<PathBuf>,
    scheduler_path: PathBuf,
    provider: String,
    base_url: Option<String>,
    model_override: Option<String>,
    keystore_service: Option<String>,
    app_data_dir: PathBuf,
}

/// 首启语料物化的进度事件。
///
/// 与桌面 `fetch_corpus` 送出的形状**逐字对齐**（`crates/yunjian-app/src/ipc.rs`
/// 的 `CorpusProgress`）：两个外壳读同一份内核事件，形状分叉会让两边的界面文案
/// 无法互相参照，而移动端的判词正是照桌面那份写的。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
enum NativeCorpusProgress {
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
        step: String,
        done: u64,
        total: u64,
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

impl From<yunjian_core::MaterializationProgress<'_>> for NativeCorpusProgress {
    fn from(progress: yunjian_core::MaterializationProgress<'_>) -> Self {
        use yunjian_core::MaterializationProgress as Source;
        match progress {
            Source::AlreadyPresent { path } => Self::AlreadyPresent {
                path: path.display().to_string(),
            },
            Source::VerifyingArchive { archive, bytes } => Self::VerifyingArchive {
                archive: archive.display().to_string(),
                bytes,
            },
            Source::ArchiveVerified { sha256 } => Self::ArchiveVerified {
                sha256: sha256.to_owned(),
            },
            Source::Decompressing {
                bytes_done,
                bytes_total,
            } => Self::Decompressing {
                bytes_done,
                bytes_total,
            },
            Source::Materialized {
                path,
                corpus_version,
            } => Self::Materialized {
                path: path.display().to_string(),
                corpus_version: corpus_version.to_owned(),
            },
            // 送内核给的中文步骤名与 done/total 三个字段，而不是 `format!("{progress:?}")`：
            // 后者把三样揉成一句 Rust 调试串，界面除了原样打印别无选择。桌面在
            // PR #108 修过同一处。
            Source::Deriving(progress) => Self::Deriving {
                step: progress.step.display_name().to_owned(),
                done: progress.done,
                total: progress.total,
            },
            Source::DeriveFailed { reason } => Self::DeriveFailed {
                reason: reason.to_owned(),
            },
            Source::Ready {
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

/// 走生产路径下载、校验并原子物化语料与随包赏析种子，返回可拉取进度的句柄。
///
/// # 为什么是顶层函数而不是 [`NativeFacade`] 的方法
///
/// [`NativeFacade::new`] 要打开语料才能构造；首启时语料还不存在，于是「先构造门面再让它
/// 去取语料」在时序上不成立。宿主的正确顺序是：`materialize_assets` -> 等到 `Done` ->
/// 再构造门面。
///
/// 调用方仍需先完成 `YunjianAndroid.initialize(context)`：赏析种子要写进
/// `appreciation.db`，而那条路径与钥匙串同处一个应用私有目录。
#[uniffi::export]
pub fn materialize_assets(config_json: String) -> Result<Arc<NativeOperation>, NativeError> {
    ensure_android_context()?;
    let config: NativeFacadeConfig = decode_json(&config_json, "NativeFacadeConfig")?;
    let corpus_config = CorpusConfig {
        path: config.corpus_path,
        data_dir: config.corpus_data_dir,
        archive: config.corpus_archive,
    };
    let app_data_dir = config.app_data_dir;
    let handle = yunjian_core::operation::start_operation(move |reporter| {
        let mut cancelled = false;
        let synced = yunjian_ai::sync_shipped_assets_with_progress(
            corpus_config,
            &app_data_dir,
            &mut |progress| {
                if reporter.is_cancelled() || reporter.is_closed() {
                    cancelled = true;
                    return;
                }
                reporter.progress(NativeCorpusProgress::from(progress));
            },
        );
        if cancelled {
            return Ok(());
        }
        match synced {
            Ok(assets) => {
                reporter.item(ShippedAssetsSummary {
                    corpus_version: assets.corpus.meta().corpus_version.clone(),
                    poem_count: assets.corpus.meta().poem_count,
                    shipped_records: assets.seed.record_count,
                    stale_records: assets.seed.stale_count,
                    seed_path: assets.seed_path.display().to_string(),
                });
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    });
    Ok(NativeOperation::from_operation(handle))
}

/// 随包赏析的展示载荷。
///
/// `reviewed` 恒为 `false` 并且**没有可以写 `true` 的入口**：随包数据集由
/// `xtask pregenerate` 生成，那条路径把该字段钉死在 `false`（未经人工审校）。
/// 留一个可写的字段等于允许某天有人把它翻成 `true` 而无人审校。
#[derive(Debug, Clone, Serialize)]
struct ShippedAppreciationOut {
    text: String,
    model: String,
    provider: String,
    generated_at: u64,
    template_version: String,
    grounding_digest: String,
    source: &'static str,
    reviewed: bool,
}

/// 按需下载并校验一个语音模型，返回它在设备上的目录。
///
/// # 为什么必须由 Rust 侧下载，而不是由外部工具把权重塞进来
///
/// 权重是**按需下载**的（安装包不含任何权重，见 `LICENSES.md`），下载后要逐字节校验
/// SHA-256 才算就位。真机上试过让 `adb push` 直接放进应用的外部私有目录：文件属主是
/// `shell`，应用（另一个 uid）读不到，`isDirectory` 报 `false`，看起来像「权重没推上去」，
/// 实际是推上去了但读不了。走产品自己这条路径既没有属主问题，也顺带证明了按需下载可用。
///
/// `cache_root` 由 `YUNJIAN_MODEL_DIR` 决定，而 Android 上那个默认值指向编译期的仓库
/// 路径、在设备上不存在，所以根目录必须由宿主显式给出。
///
/// 与 [`materialize_assets`] 同样返回可拉进度的句柄：whisper tiny 116 MiB、
/// streaming zipformer 531 MiB，都不是能让界面干等的量级。
#[cfg(feature = "native-voice")]
#[uniffi::export]
pub fn fetch_voice_model(
    cache_root: String,
    model_name: String,
) -> Result<Arc<NativeOperation>, NativeError> {
    ensure_android_context()?;
    let handle = yunjian_core::operation::start_operation(move |reporter| {
        let cache = yunjian_voice::models::ModelCache::at(&cache_root);
        let mut cancelled = false;
        let outcome = cache.ensure(&model_name, &mut |progress| {
            if reporter.is_cancelled() || reporter.is_closed() {
                cancelled = true;
                return;
            }
            reporter.progress(NativeModelProgress::from(progress));
        });
        if cancelled {
            return Ok(());
        }
        match outcome {
            Ok(dir) => {
                reporter.item(NativeModelReady {
                    directory: dir.display().to_string(),
                });
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    });
    Ok(NativeOperation::from_operation(handle))
}

/// 模型下载进度。四个变体与 [`yunjian_voice::models::FetchProgress`] 一一对应。
#[cfg(feature = "native-voice")]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
enum NativeModelProgress {
    Downloading { bytes_done: u64, bytes_total: u64 },
    Verifying { bytes: u64 },
    Verified,
    Unpacking,
}

#[cfg(feature = "native-voice")]
impl From<yunjian_voice::models::FetchProgress> for NativeModelProgress {
    fn from(progress: yunjian_voice::models::FetchProgress) -> Self {
        use yunjian_voice::models::FetchProgress as Source;
        match progress {
            Source::Downloading {
                bytes_done,
                bytes_total,
            } => Self::Downloading {
                bytes_done,
                bytes_total,
            },
            Source::Verifying { bytes } => Self::Verifying { bytes },
            Source::Verified => Self::Verified,
            Source::Unpacking => Self::Unpacking,
        }
    }
}

/// 模型已就位。
#[cfg(feature = "native-voice")]
#[derive(Debug, Clone, Serialize)]
struct NativeModelReady {
    directory: String,
}

/// 物化完成后交给宿主的事实摘要。
#[derive(Debug, Clone, Serialize)]
struct ShippedAssetsSummary {
    corpus_version: String,
    poem_count: i64,
    shipped_records: usize,
    stale_records: usize,
    seed_path: String,
}

/// Kotlin/Swift 可直接构造的生产门面。
#[derive(uniffi::Object)]
pub struct NativeFacade {
    inner: MobileFacade,
    runtime: tokio::runtime::Runtime,
    provider: GenAiProviderConfig,
    keystore: KeyStoreConfig,
    corpus_status_json: String,
    app_data_dir: PathBuf,
    corpus_version: String,
    configured_provider: String,
}

#[uniffi::export]
impl NativeFacade {
    /// 打开语料、复习库与系统钥匙串并构造共享门面。
    ///
    /// Android 宿主必须先调用 Kotlin 的 `YunjianAndroid.initialize(context)`；该检查发生在
    /// [`KeyStore::open`] 之前，避免底层 store 以未初始化的全局 context 进入 JNI。
    #[uniffi::constructor]
    pub fn new(config_json: String) -> Result<Arc<Self>, NativeError> {
        ensure_android_context()?;
        let config: NativeFacadeConfig = decode_json(&config_json, "NativeFacadeConfig")?;
        let corpus_config = CorpusConfig {
            path: config.corpus_path,
            data_dir: config.corpus_data_dir,
            archive: config.corpus_archive,
        };
        let corpus = CorpusHandle::open(&corpus_config).map_err(native_error)?;
        let corpus_version = corpus.meta().corpus_version.clone();
        let corpus_status_json = encode_json(&MobileFacade::corpus_status(&corpus))?;
        let scheduler = Scheduler::open(config.scheduler_path).map_err(native_error)?;
        // `none` 意为「不配置生成供应商」，它**不是** `ProviderKind` 的取值。
        // 直接 `parse` 会让整个门面在首启时构造失败（真机实测：
        // 「配置错误：未知的 AI 供应商 none」），于是检索、阅读、背诵全都用不了——
        // 而这三件与 AI 供应商毫无关系。桌面在 `prepare_appreciation` 里同样把
        // `PROVIDER_NONE` 挡在 parse 之前。
        //
        // 落到一个占位 kind 上只为让 `GenAiProviderConfig` 有值；随包赏析走
        // `shipped_appreciation`（不碰 provider），生成路径由 `appreciate` 自己
        // 在缺 key 时报错。
        let configured = config.provider.trim();
        let kind = if configured.eq_ignore_ascii_case(yunjian_core::config::PROVIDER_NONE) {
            ProviderKind::OpenAI
        } else {
            ProviderKind::parse(configured).map_err(native_error)?
        };
        let mut provider = GenAiProviderConfig::new(kind);
        if let Some(base_url) = config.base_url {
            provider = provider.with_base_url(base_url);
        }
        if let Some(model) = config.model_override {
            provider = provider.with_model_override(model);
        }
        let mut keystore = KeyStoreConfig::default();
        if let Some(service) = config.keystore_service {
            keystore.service = service;
        }
        let store = KeyStore::open(keystore.clone()).map_err(native_error)?;
        let null_provider = NullProvider::new(kind.as_str()).map_err(native_error)?;
        let inner = MobileFacade::new(
            corpus,
            Arc::new(null_provider),
            scheduler,
            store,
            yunjian_core::VoiceSessionConfig::default(),
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("yunjian-uniffi-async")
            .build()
            .map_err(native_error)?;
        Ok(Arc::new(Self {
            inner,
            runtime,
            provider,
            keystore,
            corpus_status_json,
            app_data_dir: config.app_data_dir,
            corpus_version,
            configured_provider: configured.to_owned(),
        }))
    }

    /// 检索正文或残句；请求与响应沿用领域 serde 契约。
    pub fn search_text(&self, request_json: String) -> Result<String, NativeError> {
        let request: TextSearchRequest = decode_json(&request_json, "TextSearchRequest")?;
        encode_json(&self.inner.search_text(request).map_err(native_error)?)
    }

    /// 读取作品详情；请求与响应沿用领域 serde 契约。
    pub fn poem_detail(&self, request_json: String) -> Result<String, NativeError> {
        let request: PoemDetailRequest = decode_json(&request_json, "PoemDetailRequest")?;
        encode_json(&self.inner.poem_detail(request).map_err(native_error)?)
    }

    /// 返回已打开语料的状态。
    pub fn corpus_status(&self) -> Result<String, NativeError> {
        Ok(self.corpus_status_json.clone())
    }

    /// 开始一次打字背诵。
    pub fn recite_start(&self, request_json: String) -> Result<String, NativeError> {
        let request: ReciteStartRequest = decode_json(&request_json, "ReciteStartRequest")?;
        encode_json(&self.inner.recite_start(request).map_err(native_error)?)
    }

    /// 评分并提交一次打字背诵。
    pub fn recite_submit(&self, request_json: String) -> Result<String, NativeError> {
        let request: ReciteSubmitRequest = decode_json(&request_json, "ReciteSubmitRequest")?;
        encode_json(&self.inner.recite_submit(request).map_err(native_error)?)
    }

    /// 返回今天到期的背诵项目。
    pub fn recite_due(&self) -> Result<String, NativeError> {
        encode_json(&self.inner.recite_due().map_err(native_error)?)
    }

    /// 返回凭据存储状态，不返回秘密材料。
    pub fn keystore_status(&self, account: String) -> Result<String, NativeError> {
        ensure_android_context()?;
        encode_json(&self.inner.keystore_status(&account).map_err(native_error)?)
    }

    /// 写入凭据并只返回非机密存储描述。
    pub fn keystore_set(&self, account: String, secret: String) -> Result<String, NativeError> {
        ensure_android_context()?;
        encode_json(
            &self
                .inner
                .keystore_set(&account, &secret)
                .map_err(native_error)?,
        )
    }

    /// 删除凭据。
    pub fn keystore_delete(&self, account: String) -> Result<String, NativeError> {
        ensure_android_context()?;
        encode_json(&self.inner.keystore_delete(&account).map_err(native_error)?)
    }

    /// 读取随包赏析；命中时**不需要也不触碰** API key。
    ///
    /// # 为什么必须与 [`Self::appreciate`] 分开
    ///
    /// 随包赏析是一份已经随发布物交付的数据，取它只需要读本地 `appreciation.db`。
    /// 把它藏在生成路径后面会让「没配 key 就看不到随包赏析」成为事实——桌面端一度
    /// 如此，靠 `prepare_appreciation` 把 `AppreciationCache::lookup` 提到 key 检查
    /// **之前**才修好（`crates/yunjian-app/src/ipc.rs`）。这里照同一顺序接线，并且
    /// 因为是两个独立方法，宿主不可能"顺手"要求一个 key。
    ///
    /// 返回 `None` 表示这首诗没有随包赏析，不是错误。
    pub fn shipped_appreciation(
        &self,
        poem_id: String,
        model: String,
    ) -> Result<Option<String>, NativeError> {
        let detail = self
            .inner
            .poem_detail(PoemDetailRequest { poem_id })
            .map_err(native_error)?;
        let request = AppreciationRequest::new(detail, model);
        let cache = AppreciationCache::open(
            &self.app_data_dir,
            self.corpus_version.clone(),
            DEFAULT_APPRECIATION_CACHE_CAPACITY,
        )
        .map_err(native_error)?;
        let lookup_provider =
            ProviderId::new(self.configured_provider.as_str()).map_err(native_error)?;
        let Some(hit) = cache
            .lookup(&request, &lookup_provider)
            .map_err(native_error)?
        else {
            return Ok(None);
        };
        encode_json(&ShippedAppreciationOut {
            text: hit.appreciation.text,
            model: hit.appreciation.model,
            provider: hit.appreciation.provider.as_str().to_owned(),
            generated_at: hit.appreciation.generated_at,
            template_version: hit.appreciation.template_version,
            grounding_digest: hit.appreciation.grounding_digest,
            source: match hit.source {
                CacheSource::Shipped => "shipped",
                CacheSource::Local => "cache",
                CacheSource::Generated => "generated",
            },
            reviewed: false,
        })
        .map(Some)
    }

    /// 生成完整 AI 赏析。
    pub fn appreciate(
        &self,
        poem_id: String,
        model: String,
        style: Option<String>,
    ) -> Result<String, NativeError> {
        let (provider, request) = self.appreciation_request(poem_id, model, style)?;
        let appreciation = self
            .runtime
            .block_on(provider.appreciate(request))
            .map_err(native_error)?;
        encode_json(&appreciation)
    }

    /// 启动流式 AI 赏析，返回支持 pull、callback、cancel 与 close 的统一句柄。
    pub fn appreciate_stream(
        &self,
        poem_id: String,
        model: String,
        style: Option<String>,
    ) -> Result<Arc<NativeOperation>, NativeError> {
        let (provider, request) = self.appreciation_request(poem_id, model, style)?;
        let handle = self
            .runtime
            .block_on(provider.appreciate_stream(request))
            .map_err(native_error)?;
        Ok(NativeOperation::from_operation(handle))
    }

    /// 启动真实 sherpa 双路 ASR；原生采集层随后用 [`NativeAsrOperation::push_pcm`]
    /// 逐帧送入单声道 `f32` PCM。
    pub fn start_asr(
        &self,
        model_dir: String,
        int8: bool,
        reference: String,
        sample_rate: u32,
    ) -> Result<Arc<NativeAsrOperation>, NativeError> {
        NativeAsrOperation::start(PathBuf::from(model_dir), int8, reference, sample_rate)
    }
}

impl NativeFacade {
    fn appreciation_request(
        &self,
        poem_id: String,
        model: String,
        style: Option<String>,
    ) -> Result<(GenAiProvider, AppreciationRequest), NativeError> {
        ensure_android_context()?;
        let detail = self
            .inner
            .poem_detail(PoemDetailRequest { poem_id })
            .map_err(native_error)?;
        let store = KeyStore::open(self.keystore.clone()).map_err(native_error)?;
        let provider =
            GenAiProvider::from_keystore(self.provider.clone(), &store).map_err(native_error)?;
        Ok((
            provider,
            AppreciationRequest::new(detail, model).with_style(style),
        ))
    }
}

#[cfg(feature = "native-voice")]
#[derive(Debug, Clone, Serialize)]
struct NativeAsrProgress {
    elapsed_ms: u64,
    spoke: bool,
    pause_count: usize,
}

#[cfg(feature = "native-voice")]
impl From<RecognitionProgress> for NativeAsrProgress {
    fn from(value: RecognitionProgress) -> Self {
        Self {
            elapsed_ms: value.elapsed_ms,
            spoke: value.spoke,
            pause_count: value.pause_count,
        }
    }
}

#[cfg(feature = "native-voice")]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NativeAsrItem {
    Partial {
        at_ms: u64,
        unbiased: Option<String>,
        biased: Option<String>,
    },
    Prompt {
        next_chars: String,
        from_index: usize,
        at_ms: u64,
        reason: &'static str,
    },
    Outcome {
        spoke: bool,
        pause_count: usize,
        long_pause_count: usize,
        onsets_ms: Vec<u64>,
        total_ms: u64,
        prompt_count: usize,
        single_rtf: f32,
        dual_rtf: f32,
        biased_enabled: bool,
        highlighting_enabled: bool,
    },
}

#[cfg(feature = "native-voice")]
impl From<RecognitionItem> for NativeAsrItem {
    fn from(value: RecognitionItem) -> Self {
        match value {
            RecognitionItem::Partial(value) => Self::from_partial(value),
            RecognitionItem::Prompt(value) => Self::from_prompt(value),
            RecognitionItem::Outcome(value) => Self::from_outcome(value),
        }
    }
}

#[cfg(feature = "native-voice")]
impl NativeAsrItem {
    fn from_partial(value: PartialHypothesis) -> Self {
        Self::Partial {
            at_ms: value.at_ms,
            unbiased: value
                .unbiased
                .map(|hypothesis| hypothesis.as_str().to_owned()),
            biased: value
                .biased
                .map(|hypothesis| hypothesis.as_str().to_owned()),
        }
    }

    const fn prompt_reason(reason: PromptReason) -> &'static str {
        match reason {
            PromptReason::NoSpeechYet => "no_speech_yet",
            PromptReason::TrailingSilence => "trailing_silence",
        }
    }

    fn from_prompt(value: Prompt) -> Self {
        Self::Prompt {
            next_chars: value.next_chars,
            from_index: value.from_index,
            at_ms: value.at_ms,
            reason: Self::prompt_reason(value.reason),
        }
    }

    fn from_outcome(value: RecognitionOutcome) -> Self {
        Self::Outcome {
            spoke: value.spoke,
            pause_count: value.pause_count,
            long_pause_count: value.long_pause_count,
            onsets_ms: value.onsets_ms,
            total_ms: value.total_ms,
            prompt_count: value.prompt_count,
            single_rtf: value.cost.single.value(),
            dual_rtf: value.cost.dual.value(),
            biased_enabled: value.plan.runs_biased(),
            highlighting_enabled: value.plan.highlighting(),
        }
    }
}

#[cfg(feature = "native-voice")]
struct NativePcmSource {
    receiver: Receiver<Vec<f32>>,
    sample_rate: u32,
    stopped: Arc<AtomicBool>,
}

#[cfg(feature = "native-voice")]
impl PcmSource for NativePcmSource {
    fn next_frame(&mut self) -> Option<Vec<f32>> {
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return None;
            }
            match self.receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(samples) => return Some(samples),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// 可由原生采集层持续送入 PCM 的 ASR 操作。
#[derive(uniffi::Object)]
pub struct NativeAsrOperation {
    operation: Arc<NativeOperation>,
    #[cfg(feature = "native-voice")]
    sender: Mutex<Option<SyncSender<Vec<f32>>>>,
    #[cfg(feature = "native-voice")]
    stopped: Arc<AtomicBool>,
}

impl NativeAsrOperation {
    #[cfg(feature = "native-voice")]
    fn start(
        model_dir: PathBuf,
        int8: bool,
        reference: String,
        sample_rate: u32,
    ) -> Result<Arc<Self>, NativeError> {
        if sample_rate == 0 {
            return Err(NativeError::message("ASR sample_rate 不得为 0"));
        }
        if reference
            .chars()
            .all(|character| !character.is_alphabetic())
        {
            return Err(NativeError::message("ASR reference 不得为空"));
        }
        let files = TransducerFiles::discover(&model_dir, int8).map_err(native_error)?;
        let biased = Hotwords::from_poem(&reference).map(OnlineDecodeConfig::biased);
        let decoder = StreamingDualDecoder::open(&files, OnlineDecodeConfig::unbiased(), biased)
            .map_err(native_error)?;
        let (sender, receiver) = sync_channel(ASR_FRAME_CAPACITY);
        let stopped = Arc::new(AtomicBool::new(false));
        let source = NativePcmSource {
            receiver,
            sample_rate,
            stopped: Arc::clone(&stopped),
        };
        let mut plan = RecognitionPlan::guided(reference);
        plan.diagnostics = true;
        let recognition = start_recognition(source, decoder, plan);
        let mapped = start_operation(move |reporter| {
            loop {
                if reporter.is_cancelled() || reporter.is_closed() {
                    cancel(&recognition);
                    return Ok(());
                }
                match next_event(&recognition, 50) {
                    Some(Event::Progress(progress)) => {
                        reporter.progress(NativeAsrProgress::from(progress));
                    }
                    Some(Event::Item(item)) => {
                        if !reporter.item(NativeAsrItem::from(item)) {
                            cancel(&recognition);
                            return Ok(());
                        }
                    }
                    Some(Event::Done) => return Ok(()),
                    Some(Event::Cancelled) => return Ok(()),
                    Some(Event::Failed { message }) => return Err(message),
                    None => {}
                }
            }
        });
        Ok(Arc::new(Self {
            operation: NativeOperation::from_operation(mapped),
            sender: Mutex::new(Some(sender)),
            stopped,
        }))
    }

    #[cfg(not(feature = "native-voice"))]
    fn start(
        _model_dir: PathBuf,
        _int8: bool,
        _reference: String,
        _sample_rate: u32,
    ) -> Result<Arc<Self>, NativeError> {
        Err(NativeError::message(
            "当前原生库未启用 native-voice；请用 --features native-voice 构建",
        ))
    }

    #[cfg(feature = "native-voice")]
    fn finish_sender(&self) {
        let mut sender = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sender.take();
    }
}

#[uniffi::export]
impl NativeAsrOperation {
    /// 以有界背压队列送入一帧单声道 `f32` PCM；输入结束后不可再送。
    pub fn push_pcm(&self, samples: Vec<f32>) -> Result<(), NativeError> {
        #[cfg(not(feature = "native-voice"))]
        {
            let _ = samples;
            return Err(NativeError::message(
                "当前原生库未启用 native-voice；无法接收 PCM",
            ));
        }
        #[cfg(feature = "native-voice")]
        {
            if samples.is_empty() {
                return Err(NativeError::message("PCM 帧不得为空"));
            }
            if samples.iter().any(|sample| !sample.is_finite()) {
                return Err(NativeError::message("PCM 帧含非有限采样"));
            }
            let sender = self
                .sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .ok_or_else(|| NativeError::message("ASR 输入已经结束"))?;
            sender
                .send(samples)
                .map_err(|_| NativeError::message("ASR 生产者已经结束"))
        }
    }

    /// 标记 PCM 输入结束；识别器会刷新最终 partial 与 outcome。
    pub fn finish_input(&self) {
        #[cfg(feature = "native-voice")]
        self.finish_sender();
    }

    /// 在超时内拉取下一条 ASR 事件。
    pub fn next_event(&self, timeout_ms: u64) -> Option<String> {
        self.operation.next_event(timeout_ms)
    }

    /// 在后台线程按序回调 ASR 事件。
    pub fn subscribe(&self, sink: Box<dyn NativeEventSink>) {
        Arc::clone(&self.operation).subscribe(sink);
    }

    /// 请求取消并唤醒正在等待 PCM 的识别线程。
    pub fn cancel(&self) {
        #[cfg(feature = "native-voice")]
        {
            self.stopped.store(true, Ordering::Release);
        }
        self.operation.cancel();
        #[cfg(feature = "native-voice")]
        self.finish_sender();
    }

    /// 关闭句柄、释放事件并唤醒正在等待 PCM 的识别线程。
    ///
    /// 命名理由同 [`NativeOperation::shutdown`]：`close` 会与生成的 Kotlin
    /// `AutoCloseable.close()` 撞成 `Conflicting overloads`。
    pub fn shutdown(&self) {
        #[cfg(feature = "native-voice")]
        {
            self.stopped.store(true, Ordering::Release);
        }
        self.operation.shutdown();
        #[cfg(feature = "native-voice")]
        self.finish_sender();
    }
}

#[cfg(target_os = "android")]
static ANDROID_CONTEXT_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "android")]
static ANDROID_APPLICATION_CONTEXT: std::sync::OnceLock<jni::objects::GlobalRef> =
    std::sync::OnceLock::new();

/// JNI 入口：Kotlin 的 `YunjianAndroid.initialize(context)` 在构造门面前调用。
///
/// 全局引用必须存活到进程结束，因为 `ndk-context` 只借用它；传入 Activity 时 Kotlin
/// 包装层会先转换成 application context，避免 Activity 销毁后留下悬垂引用。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_top_yunjian_mobile_YunjianAndroid_initializeNative(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    context: jni::objects::JObject<'_>,
) {
    if let Err(error) = initialize_android_context_from_jni(&env, context) {
        let _ = env.throw_new("java/lang/IllegalStateException", error.to_string());
    }
}

#[cfg(target_os = "android")]
fn initialize_android_context_from_jni(
    env: &jni::JNIEnv<'_>,
    context: jni::objects::JObject<'_>,
) -> Result<(), NativeError> {
    if context.is_null() {
        return Err(NativeError::message("Android application context 不得为空"));
    }
    if ANDROID_CONTEXT_INITIALIZED.load(Ordering::Acquire) {
        return Ok(());
    }
    let java_vm = env.get_java_vm().map_err(native_error)?;
    let global = env.new_global_ref(context).map_err(native_error)?;
    let application_context = global.as_obj().as_raw().cast();
    ANDROID_APPLICATION_CONTEXT
        .set(global)
        .map_err(|_| NativeError::message("Android context 已经初始化"))?;
    // SAFETY: `JavaVM` 来自当前有效 JNIEnv；jobject 已提升为上方 OnceLock 持有的 GlobalRef，
    // 两个指针在进程生命周期内有效，且 OnceLock 保证底层全局只初始化一次。
    unsafe {
        ndk_context::initialize_android_context(
            java_vm.get_java_vm_pointer().cast(),
            application_context,
        );
    }
    ANDROID_CONTEXT_INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

#[cfg(target_os = "android")]
fn ensure_android_context() -> Result<(), NativeError> {
    ANDROID_CONTEXT_INITIALIZED
        .load(Ordering::Acquire)
        .then_some(())
        .ok_or_else(|| {
            NativeError::message(
                "Android context 尚未初始化；必须先调用 initialize_android_context",
            )
        })
}

#[cfg(not(target_os = "android"))]
fn ensure_android_context() -> Result<(), NativeError> {
    Ok(())
}
