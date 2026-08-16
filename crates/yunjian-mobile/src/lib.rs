//! 云笺移动端共享门面。
//!
//! 此 crate 只负责把四个领域 crate 接成单一入口；移动外壳与 binding 分支必须等待
//! `docs/reports/mobile-spike.md` 给出确定裁决后再落地。

#![warn(missing_docs)]

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

#[cfg(feature = "uniffi")]
pub mod uniffi_native;

use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use yunjian_ai::{
    Appreciation, AppreciationProgress, AppreciationProvider, AppreciationRequest,
    AppreciationStreamItem, KeyStore, StorageReport,
};
use yunjian_core::operation::{Event, OperationHandle, cancel, close, next_event};
use yunjian_core::{
    Attribution, AuthorDetail, AuthorDetailRequest, AuthorSearchRequest, CharacterRhymesRequest,
    CorpusConfig, CorpusHandle, CorpusMeta, DictionaryLookup, DictionaryLookupRequest,
    DynastyBrowseRequest, FirstLineSearchRequest, LastCharacterSearchRequest, MetaPage, PoemDetail,
    PoemDetailRequest, PoemFeatures, Result, RhymeAnswer, RhymeCheckRequest, RhymeGroupMatches,
    RhymeGroupRef, RhymeGroupSearchRequest, SearchPage, TagBrowseRequest, TagSummary,
    TextSearchRequest, TitleSearchRequest, VoiceSessionConfig, WorkGroupRequest, Yunjian,
};
use yunjian_recite::{
    ClozeOptions, FsrsGrade, MaskStage, OpsSummary, PracticeMode, PracticeSession, ReviewState,
    Scheduler, TypedScore, review_typed_text,
};
use yunjian_voice::session::{
    Demonstrator, Listener, SessionItem, SessionPlan, SessionProgress, SessionScript, start_session,
};

/// 移动框架 spike 的三态裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingVerdict {
    /// 使用 Tauri mobile。
    TauriMobile,
    /// 使用 UniFFI 原生外壳。
    UniffiNative,
    /// 真机证据不足，暂不构建任何 binding。
    Undetermined,
}

/// 当前权威报告（`docs/reports/mobile-spike.md`）记录的裁决。
///
/// 2026-08-16 由 `Undetermined` 变为 `UniffiNative`：判据②语料物化在物理 Pixel 8 上实测
/// `duration_seconds=109.849`，超过预声明的 60 秒阈值，机械规则据此选择 UniFFI 原生外壳。
/// **改这个常量前先看报告**，它必须与报告逐字一致，否则 `surface.rs` 的守卫会红。
pub const BINDING_VERDICT: BindingVerdict = BindingVerdict::UniffiNative;

/// 当前是否构建了 Tauri mobile binding。
pub const TAURI_MOBILE_BINDING: bool = false;

/// 当前是否构建了 UniFFI native binding。
///
/// Kotlin、Swift 生成物和 Android 初始化包装器均由 `yunjian-mobile` 提供；
/// `tests/architecture.rs` 会阻止常量、feature 与版本化产物再次失配。
pub const UNIFFI_NATIVE_BINDING: bool = true;

/// 移动端可选择的打字练习形态。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ReciteMode {
    /// 按比例挖空。
    Cloze {
        /// 挖空比例。
        ratio: f32,
        /// 确定性选位种子。
        seed: u64,
    },
    /// 每行只显示首字。
    FirstChar,
    /// 遮住前若干行。
    Masked {
        /// 已遮住的行数。
        masked_lines: usize,
    },
}

impl From<ReciteMode> for PracticeMode {
    fn from(value: ReciteMode) -> Self {
        match value {
            ReciteMode::Cloze { ratio, seed } => Self::Cloze(ClozeOptions::new(ratio, seed)),
            ReciteMode::FirstChar => Self::FirstChar,
            ReciteMode::Masked { masked_lines } => Self::Masked(MaskStage::new(masked_lines)),
        }
    }
}

impl From<PracticeMode> for ReciteMode {
    fn from(value: PracticeMode) -> Self {
        match value {
            PracticeMode::Cloze(options) => Self::Cloze {
                ratio: options.ratio(),
                seed: options.seed(),
            },
            PracticeMode::FirstChar => Self::FirstChar,
            PracticeMode::Masked(stage) => Self::Masked {
                masked_lines: stage.masked_lines(),
            },
        }
    }
}

/// 开始一次打字背诵的请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReciteStartRequest {
    /// 语料中的稳定作品标识。
    pub poem_id: String,
    /// 练习形态。
    pub mode: ReciteMode,
}

/// 一次打字背诵题目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReciteSession {
    /// 语料中的稳定作品标识。
    pub poem_id: String,
    /// 实际采用的练习形态。
    pub mode: ReciteMode,
    /// 保留原断句的提示文本。
    pub prompt: String,
    /// 被遮位置。
    pub hidden_indices: Vec<usize>,
    /// 呈现行数。
    pub line_count: usize,
    /// 遮挡形态总档位数。
    pub stage_count: usize,
}

/// 提交一次打字背诵的请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReciteSubmitRequest {
    /// 语料中的稳定作品标识。
    pub poem_id: String,
    /// 用户键入的答案。
    pub answer: String,
    /// 用户确认提交的 FSRS 等级。
    pub grade: ReciteGrade,
}

/// 移动边界使用的 FSRS 等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReciteGrade {
    /// 未能回忆。
    Again,
    /// 回忆困难。
    Hard,
    /// 正常回忆。
    Good,
    /// 轻松回忆。
    Easy,
}

impl From<ReciteGrade> for FsrsGrade {
    fn from(value: ReciteGrade) -> Self {
        match value {
            ReciteGrade::Again => Self::Again,
            ReciteGrade::Hard => Self::Hard,
            ReciteGrade::Good => Self::Good,
            ReciteGrade::Easy => Self::Easy,
        }
    }
}

/// 一次打字背诵评分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReciteScore {
    /// 未漏字符比例。
    pub completeness: f32,
    /// 严格准确度。
    pub accuracy_strict: f32,
    /// 近音宽容后的准确度。
    pub accuracy_lenient: f32,
    /// 打字路径的中性流利度值。
    pub fluency: f32,
    /// 是否拒绝本次答案。
    pub is_rejected: bool,
    /// 正常匹配数。
    pub normal_count: usize,
    /// 漏读数。
    pub deletion_count: usize,
    /// 增读数。
    pub insertion_count: usize,
    /// 回读片段数。
    pub rerecitation_count: usize,
    /// 替换数。
    pub substitution_count: usize,
}

impl From<TypedScore> for ReciteScore {
    fn from(score: TypedScore) -> Self {
        let OpsSummary {
            normal_count,
            deletion_count,
            insertion_count,
            rerecitation_count,
            substitution_count,
        } = score.ops_summary;
        Self {
            completeness: score.completeness,
            accuracy_strict: score.accuracy_strict,
            accuracy_lenient: score.accuracy_lenient,
            fluency: score.fluency,
            is_rejected: score.is_rejected,
            normal_count,
            deletion_count,
            insertion_count,
            rerecitation_count,
            substitution_count,
        }
    }
}

/// 移动边界使用的复习状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReciteReview {
    /// 语料中的稳定作品标识。
    pub stable_id: String,
    /// 当前记忆稳定度。
    pub stability: f32,
    /// 当前记忆难度。
    pub difficulty: f32,
    /// 下次到期的 Unix 日序号。
    pub due_day: i64,
    /// 最近复习的 Unix 日序号。
    pub last_review_day: i64,
    /// 最近一次间隔天数。
    pub scheduled_days: u32,
    /// 最近提交的等级。
    pub last_grade: ReciteGrade,
}

impl From<ReviewState> for ReciteReview {
    fn from(state: ReviewState) -> Self {
        Self {
            stable_id: state.stable_id,
            stability: state.stability,
            difficulty: state.difficulty,
            due_day: state.due_day,
            last_review_day: state.last_review_day,
            scheduled_days: state.scheduled_days,
            last_grade: match state.last_grade {
                FsrsGrade::Again => ReciteGrade::Again,
                FsrsGrade::Hard => ReciteGrade::Hard,
                FsrsGrade::Good => ReciteGrade::Good,
                FsrsGrade::Easy => ReciteGrade::Easy,
            },
        }
    }
}

/// 打字背诵提交结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReciteSubmission {
    /// 归一化后的答案。
    pub normalized_answer: String,
    /// 领域评分结果。
    pub score: ReciteScore,
    /// 提交后复习状态。
    pub review: ReciteReview,
}

/// 启动语音跟读会话的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceSessionRequest {
    /// 带断句的作品正文。
    pub body: String,
    /// 是否先播放逐行示范。
    pub demonstrate: bool,
}

/// 凭据状态；不含密钥材料。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyStatus {
    /// 当前或被查询层的非机密描述。
    pub report: StorageReport,
    /// 是否需要重新询问用户。
    pub needs_reprompt: bool,
}

/// 可展示的语料状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusStatus {
    /// schema 版本。
    pub schema_version: u32,
    /// 语料版本。
    pub corpus_version: String,
    /// 构建时间。
    pub built_at: String,
    /// 作品数量。
    pub poem_count: i64,
    /// FTS detail 模式。
    pub index_detail_mode: String,
    /// 派生索引分发策略。
    pub derived_indexes: String,
    /// 随包范围。
    pub shipped_scope: String,
    /// 派生结构是否就绪。
    pub derived_ready: bool,
}

impl CorpusStatus {
    fn from_handle(corpus: &CorpusHandle) -> Self {
        let CorpusMeta {
            schema_version,
            corpus_version,
            built_at,
            poem_count,
            index_detail_mode,
            derived_indexes,
            shipped_scope,
        } = corpus.meta().clone();
        Self {
            schema_version,
            corpus_version,
            built_at,
            poem_count,
            index_detail_mode,
            derived_indexes,
            shipped_scope,
            derived_ready: corpus.derived().is_ready(),
        }
    }
}

/// 面向移动外壳的单一共享门面。
pub struct MobileFacade {
    corpus: CorpusHandle,
    core: Yunjian,
    appreciation: Arc<dyn AppreciationProvider>,
    scheduler: Mutex<Scheduler>,
    keystore: KeyStore,
    voice_config: VoiceSessionConfig,
}

impl MobileFacade {
    /// 接管四个领域 crate 的运行时对象。
    #[must_use]
    pub fn new(
        corpus: CorpusHandle,
        appreciation: Arc<dyn AppreciationProvider>,
        scheduler: Scheduler,
        keystore: KeyStore,
        voice_config: VoiceSessionConfig,
    ) -> Self {
        Self {
            core: Yunjian::new(corpus.clone()),
            corpus,
            appreciation,
            scheduler: Mutex::new(scheduler),
            keystore,
            voice_config,
        }
    }

    /// 检索正文或残句。
    pub fn search_text(&self, request: TextSearchRequest) -> Result<SearchPage> {
        self.core.search_text(request)
    }

    /// 按题目检索。
    pub fn find_by_title(&self, request: TitleSearchRequest) -> Result<MetaPage> {
        self.core.find_by_title(request)
    }

    /// 按作者检索。
    pub fn find_by_author(&self, request: AuthorSearchRequest) -> Result<MetaPage> {
        self.core.find_by_author(request)
    }

    /// 读取作者详情。
    pub fn author_detail(&self, request: AuthorDetailRequest) -> Result<AuthorDetail> {
        self.core.author_detail(request)
    }

    /// 按朝代浏览。
    pub fn browse_by_dynasty(&self, request: DynastyBrowseRequest) -> Result<MetaPage> {
        self.core.browse_by_dynasty(request)
    }

    /// 按首句检索。
    pub fn find_by_first_line(&self, request: FirstLineSearchRequest) -> Result<MetaPage> {
        self.core.find_by_first_line(request)
    }

    /// 按句末字检索。
    pub fn find_by_last_character(&self, request: LastCharacterSearchRequest) -> Result<MetaPage> {
        self.core.find_by_last_character(request)
    }

    /// 读取作品分组的全部归属。
    pub fn work_group_attributions(&self, request: WorkGroupRequest) -> Result<Vec<Attribution>> {
        self.core.work_group_attributions(request)
    }

    /// 按韵部检索。
    pub fn find_by_rhyme_group(
        &self,
        request: RhymeGroupSearchRequest,
    ) -> Result<RhymeGroupMatches> {
        self.core.find_by_rhyme_group(request)
    }

    /// 判断多个字是否押韵。
    pub fn do_these_rhyme(&self, request: RhymeCheckRequest) -> Result<RhymeAnswer> {
        self.core.do_these_rhyme(request)
    }

    /// 查询一个字的韵部归属。
    pub fn rhyme_groups_of(&self, request: CharacterRhymesRequest) -> Result<Vec<RhymeGroupRef>> {
        self.core.rhyme_groups_of(request)
    }

    /// 列出策展标签。
    pub fn list_tags(&self) -> Result<Vec<TagSummary>> {
        self.core.list_tags()
    }

    /// 按标签浏览。
    pub fn browse_by_tag(&self, request: TagBrowseRequest) -> Result<MetaPage> {
        self.core.browse_by_tag(request)
    }

    /// 读取作品详情。
    pub fn poem_detail(&self, request: PoemDetailRequest) -> Result<PoemDetail> {
        self.core.poem_detail(request)
    }

    /// 查询内置字典。
    pub fn lookup_dictionary(&self, request: DictionaryLookupRequest) -> Result<DictionaryLookup> {
        self.core.lookup_dictionary(request)
    }

    /// 批量读取作品特征。
    pub fn poem_features(&self, poem_ids: &[&str]) -> Result<Vec<PoemFeatures>> {
        self.core.poem_features(poem_ids)
    }

    /// 生成完整 AI 赏析。
    pub async fn appreciate(&self, request: AppreciationRequest) -> Result<Appreciation> {
        self.appreciation.appreciate(request).await
    }

    /// 启动流式 AI 赏析。
    pub async fn appreciate_stream(
        &self,
        request: AppreciationRequest,
    ) -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>> {
        self.appreciation.appreciate_stream(request).await
    }

    /// 开始一次打字背诵并返回展示题目。
    pub fn recite_start(&self, request: ReciteStartRequest) -> Result<ReciteSession> {
        let detail = self.poem_detail(PoemDetailRequest {
            poem_id: request.poem_id.clone(),
        })?;
        let session = PracticeSession::start(&self.corpus, &detail.poem.body, request.mode.into())?;
        Ok(ReciteSession {
            poem_id: request.poem_id,
            mode: session.mode().into(),
            prompt: session.prompt().to_owned(),
            hidden_indices: session.hidden_indices().to_vec(),
            line_count: session.line_count(),
            stage_count: session.stage_count(),
        })
    }

    /// 评分并提交一次打字背诵。
    pub fn recite_submit(&self, request: ReciteSubmitRequest) -> Result<ReciteSubmission> {
        let detail = self.poem_detail(PoemDetailRequest {
            poem_id: request.poem_id.clone(),
        })?;
        let reference = yunjian_recite::Poem::new(&self.corpus, &detail.poem.body)?;
        let (normalized_answer, phonetic) =
            review_typed_text(&self.corpus, &reference, &request.answer)?;
        let review = self
            .scheduler()?
            .review(&request.poem_id, request.grade.into())?;
        Ok(ReciteSubmission {
            normalized_answer,
            score: phonetic.score.into(),
            review: review.into(),
        })
    }

    /// 返回今天已经到期的作品。
    pub fn recite_due(&self) -> Result<Vec<ReciteReview>> {
        self.scheduler()?
            .due_today()
            .map(|states| states.into_iter().map(ReciteReview::from).collect())
    }

    /// 启动一次跟读会话。
    pub fn voice_session_start<D, L>(
        &self,
        demonstrator: D,
        listener: L,
        request: VoiceSessionRequest,
    ) -> Result<OperationHandle<SessionProgress, SessionItem>>
    where
        D: Demonstrator + Send + 'static,
        L: Listener + Send + 'static,
    {
        let script = SessionScript::from_poem(&request.body)
            .ok_or_else(|| yunjian_core::Error::Voice("语音跟读正文不能为空".to_owned()))?;
        let mut plan = SessionPlan::guided(script, self.voice_config);
        plan.demonstrate = request.demonstrate;
        Ok(start_session(demonstrator, listener, plan))
    }

    /// 等待语音会话的下一事件。
    pub fn voice_session_next_event(
        handle: &OperationHandle<SessionProgress, SessionItem>,
        timeout_ms: u64,
    ) -> Option<Event<SessionProgress, SessionItem>> {
        next_event(handle, timeout_ms)
    }

    /// 请求取消语音会话。
    pub fn voice_session_cancel(handle: &OperationHandle<SessionProgress, SessionItem>) {
        cancel(handle);
    }

    /// 关闭语音会话并释放未消费事件。
    pub fn voice_session_close(handle: &OperationHandle<SessionProgress, SessionItem>) {
        close(handle);
    }

    /// 返回凭据存储状态，不返回密钥。
    pub fn keystore_status(&self, account: &str) -> Result<KeyStatus> {
        let lookup = self.keystore.get(account)?;
        Ok(KeyStatus {
            report: lookup.report().clone(),
            needs_reprompt: lookup.needs_reprompt(),
        })
    }

    /// 写入凭据并只返回非机密存储描述。
    pub fn keystore_set(&self, account: &str, secret: &str) -> Result<StorageReport> {
        self.keystore
            .set(account, &SecretString::from(secret.to_owned()))
    }

    /// 删除凭据。
    pub fn keystore_delete(&self, account: &str) -> Result<StorageReport> {
        self.keystore.delete(account)
    }

    /// 读取已打开语料的状态。
    #[must_use]
    pub fn corpus_status(corpus: &CorpusHandle) -> CorpusStatus {
        CorpusStatus::from_handle(corpus)
    }

    /// 必要时校验、原子解压并打开语料。
    pub fn corpus_materialize(
        config: &CorpusConfig,
        progress: &mut dyn FnMut(yunjian_core::MaterializationProgress<'_>),
    ) -> Result<CorpusHandle> {
        CorpusHandle::open_with_progress(config, progress)
    }

    fn scheduler(&self) -> Result<MutexGuard<'_, Scheduler>> {
        self.scheduler
            .lock()
            .map_err(|_| yunjian_core::Error::Recite("复习排程器锁已中毒".to_owned()))
    }
}
