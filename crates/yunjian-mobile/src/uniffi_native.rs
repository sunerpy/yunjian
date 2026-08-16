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
    AppreciationProvider, AppreciationRequest, GenAiProvider, GenAiProviderConfig, KeyStore,
    KeyStoreConfig, NullProvider, ProviderKind,
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
    pub fn close(&self) {
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
}

/// Kotlin/Swift 可直接构造的生产门面。
#[derive(uniffi::Object)]
pub struct NativeFacade {
    inner: MobileFacade,
    runtime: tokio::runtime::Runtime,
    provider: GenAiProviderConfig,
    keystore: KeyStoreConfig,
    corpus_status_json: String,
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
        let corpus_status_json = encode_json(&MobileFacade::corpus_status(&corpus))?;
        let scheduler = Scheduler::open(config.scheduler_path).map_err(native_error)?;
        let kind = ProviderKind::parse(&config.provider).map_err(native_error)?;
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
    pub fn close(&self) {
        #[cfg(feature = "native-voice")]
        {
            self.stopped.store(true, Ordering::Release);
        }
        self.operation.close();
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
