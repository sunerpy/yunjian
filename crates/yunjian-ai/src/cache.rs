//! 两级赏析缓存。

use crate::{
    Appreciation, AppreciationProgress, AppreciationProvider, AppreciationRequest,
    AppreciationStreamItem, ProviderId, TokenUsage,
};
use async_trait::async_trait;
use blake3::Hasher;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use yunjian_core::operation::{Event, OperationHandle, cancel, next_event, start_operation};
use yunjian_core::{Error, Result};

/// 应用数据目录里的赏析数据库文件名。
pub const APPRECIATION_DATABASE_FILE: &str = "appreciation.db";

/// 默认保留的用户自费生成结果数。
pub const DEFAULT_APPRECIATION_CACHE_CAPACITY: usize = 1_000;

const SCHEMA: &str = include_str!("../schema-cache.sql");
const SHIPPED_PROVIDER: &str = "shipped";

/// 缓存命中的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSource {
    /// 应用随包提供的预生成结果。
    Shipped,
    /// 用户自己的模型此前生成的结果。
    Local,
    /// 本次调用模型新生成的结果。
    Generated,
}

/// 带来源标记的赏析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheHit {
    /// 赏析正文及生成溯源。
    pub appreciation: Appreciation,
    /// 结果来自哪一级。
    pub source: CacheSource,
}

/// 两张赏析表的行数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheCounts {
    /// 随包预生成行数。
    pub shipped: usize,
    /// 用户自费生成行数。
    pub local: usize,
}

/// 一条待导入随包层的预生成赏析。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedAppreciation {
    /// 作品的稳定标识。
    pub stable_id: String,
    /// 生成时使用的模板版本。
    pub template_version: String,
    /// 生成模型。
    pub model: String,
    /// 模型权重许可。
    pub model_license: String,
    /// 生成时事实块的摘要。
    pub grounding_digest: String,
    /// 赏析正文。
    pub text: String,
    /// 生成完成时的 Unix 秒数。
    pub generated_at: u64,
}

/// 用户缓存清理范围；随包层不受影响。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurgeScope {
    /// 清理指定模板版本。
    Template(String),
    /// 清理指定作品。
    Poem(String),
    /// 清理全部用户自费缓存。
    All,
}

/// 位于应用数据目录的可写两级赏析缓存。
#[derive(Debug)]
pub struct AppreciationCache {
    path: PathBuf,
    connection: Mutex<Connection>,
    corpus_version: String,
    capacity: usize,
}

impl AppreciationCache {
    /// 在应用数据目录打开缓存并初始化签入的 schema。
    pub fn open(
        app_data_dir: impl AsRef<Path>,
        corpus_version: impl Into<String>,
        capacity: usize,
    ) -> Result<Self> {
        std::fs::create_dir_all(app_data_dir.as_ref())?;
        let path = app_data_dir.as_ref().join(APPRECIATION_DATABASE_FILE);
        let connection = Connection::open(&path)?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            corpus_version: corpus_version.into(),
            capacity,
        })
    }

    /// 返回实际打开的数据库路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 按“用户模型结果优先、随包结果其次”的顺序查找。
    pub fn lookup(
        &self,
        request: &AppreciationRequest,
        provider: &ProviderId,
    ) -> Result<Option<CacheHit>> {
        let connection = self.connection();
        let key = cache_key(request, provider);
        let local = connection
            .query_row(
                "SELECT provider, model, template_version, grounding_digest, text, created_at, tokens_in, tokens_out FROM appreciation_cache WHERE key = ?1",
                params![key.as_slice()],
                |row| {
                    let provider: String = row.get(0)?;
                    let input_tokens: Option<u32> = row.get(6)?;
                    let output_tokens: Option<u32> = row.get(7)?;
                    let usage = match (input_tokens, output_tokens) {
                        (None, None) => None,
                        (input, output) => {
                            let input_tokens = input.unwrap_or_default();
                            let output_tokens = output.unwrap_or_default();
                            Some(TokenUsage {
                                input_tokens,
                                output_tokens,
                                total_tokens: input_tokens.saturating_add(output_tokens),
                            })
                        }
                    };
                    let generated_at = nonnegative_i64(row.get(5)?, 5)?;
                    Ok(Appreciation {
                        provider: ProviderId::new(provider).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                        model: row.get(1)?,
                        template_version: row.get(2)?,
                        grounding_digest: row.get(3)?,
                        text: row.get(4)?,
                        generated_at,
                        usage,
                    })
                },
            )
            .optional()?;
        if let Some(appreciation) = local {
            return Ok(Some(CacheHit {
                appreciation,
                source: CacheSource::Local,
            }));
        }

        if request.style().is_some() {
            return Ok(None);
        }
        let stable_id = &request.poem().poem.stable_id;
        let shipped = connection
            .query_row(
                "SELECT model, grounding_digest, text, generated_at FROM appreciation_shipped WHERE stable_id = ?1 AND template_version = ?2",
                params![stable_id, request.template_version()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        nonnegative_i64(row.get(3)?, 3)?,
                    ))
                },
            )
            .optional()?;
        let Some((model, grounding_digest, text, generated_at)) = shipped else {
            return Ok(None);
        };
        if grounding_digest != request.grounding_digest() {
            connection.execute(
                "UPDATE appreciation_shipped SET stale = 1 WHERE stable_id = ?1 AND template_version = ?2",
                params![stable_id, request.template_version()],
            )?;
            return Ok(None);
        }
        connection.execute(
            "UPDATE appreciation_shipped SET stale = 0 WHERE stable_id = ?1 AND template_version = ?2",
            params![stable_id, request.template_version()],
        )?;
        Ok(Some(CacheHit {
            appreciation: Appreciation {
                text,
                model,
                provider: ProviderId::new(SHIPPED_PROVIDER)?,
                generated_at,
                template_version: request.template_version().to_owned(),
                grounding_digest,
                usage: None,
            },
            source: CacheSource::Shipped,
        }))
    }

    /// 原子写入一条完整生成结果并执行本地层 LRU 淘汰。
    pub fn store_completed(
        &self,
        request: &AppreciationRequest,
        appreciation: &Appreciation,
    ) -> Result<()> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let key = cache_key(request, &appreciation.provider);
        let (tokens_in, tokens_out) = appreciation
            .usage
            .map(|usage| (Some(usage.input_tokens), Some(usage.output_tokens)))
            .unwrap_or((None, None));
        transaction.execute(
            "INSERT OR REPLACE INTO appreciation_cache (key, stable_id, provider, model, template_version, corpus_version, grounding_digest, text, created_at, tokens_in, tokens_out) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                key.as_slice(),
                request.poem().poem.stable_id,
                appreciation.provider.as_str(),
                appreciation.model,
                appreciation.template_version,
                self.corpus_version,
                appreciation.grounding_digest,
                appreciation.text,
                i64::try_from(appreciation.generated_at).unwrap_or(i64::MAX),
                tokens_in,
                tokens_out,
            ],
        )?;
        let count =
            transaction.query_row("SELECT COUNT(*) FROM appreciation_cache", [], |row| {
                nonnegative_usize(row.get(0)?, 0)
            })?;
        let excess = count.saturating_sub(self.capacity);
        if excess > 0 {
            transaction.execute(
                "DELETE FROM appreciation_cache WHERE key IN (SELECT key FROM appreciation_cache ORDER BY created_at ASC, key ASC LIMIT ?1)",
                params![i64::try_from(excess).unwrap_or(i64::MAX)],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 导入或替换一条随包预生成结果。
    pub fn insert_shipped(&self, appreciation: &ShippedAppreciation) -> Result<()> {
        self.connection().execute(
            "INSERT OR REPLACE INTO appreciation_shipped (stable_id, template_version, model, model_license, grounding_digest, text, generated_at, stale) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![
                appreciation.stable_id,
                appreciation.template_version,
                appreciation.model,
                appreciation.model_license,
                appreciation.grounding_digest,
                appreciation.text,
                i64::try_from(appreciation.generated_at).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    /// 清理用户自费缓存并返回删除行数。
    pub fn purge(&self, scope: PurgeScope) -> Result<usize> {
        let removed = match scope {
            PurgeScope::Template(version) => self.connection().execute(
                "DELETE FROM appreciation_cache WHERE template_version = ?1",
                params![version],
            )?,
            PurgeScope::Poem(stable_id) => self.connection().execute(
                "DELETE FROM appreciation_cache WHERE stable_id = ?1",
                params![stable_id],
            )?,
            PurgeScope::All => self
                .connection()
                .execute("DELETE FROM appreciation_cache", [])?,
        };
        Ok(removed)
    }

    /// 返回两级缓存当前行数。
    pub fn counts(&self) -> Result<CacheCounts> {
        let connection = self.connection();
        Ok(CacheCounts {
            shipped: connection.query_row(
                "SELECT COUNT(*) FROM appreciation_shipped",
                [],
                |row| nonnegative_usize(row.get(0)?, 0),
            )?,
            local: connection.query_row("SELECT COUNT(*) FROM appreciation_cache", [], |row| {
                nonnegative_usize(row.get(0)?, 0)
            })?,
        })
    }

    /// 查询一条随包记录是否已因事实摘要变化而标陈旧。
    pub fn shipped_is_stale(&self, stable_id: &str, template_version: &str) -> Result<bool> {
        self.connection()
            .query_row(
                "SELECT stale FROM appreciation_shipped WHERE stable_id = ?1 AND template_version = ?2",
                params![stable_id, template_version],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 为任意赏析供应商增加两级缓存查写的装饰器。
#[derive(Debug)]
pub struct CachedAppreciationProvider<P> {
    provider: P,
    cache: Arc<AppreciationCache>,
}

impl<P> CachedAppreciationProvider<P> {
    /// 包装供应商；调用时先查用户层，再查随包层。
    #[must_use]
    pub fn new(provider: P, cache: Arc<AppreciationCache>) -> Self {
        Self { provider, cache }
    }
}

impl<P> CachedAppreciationProvider<P>
where
    P: AppreciationProvider,
{
    /// 返回结果及其来源；只有两级都未命中时才调用供应商。
    pub async fn resolve(&self, request: AppreciationRequest) -> Result<CacheHit> {
        if let Some(hit) = self.cache.lookup(&request, &self.provider.id())? {
            return Ok(hit);
        }
        let appreciation = self.provider.appreciate(request.clone()).await?;
        self.cache.store_completed(&request, &appreciation)?;
        Ok(CacheHit {
            appreciation,
            source: CacheSource::Generated,
        })
    }
}

#[async_trait]
impl<P> AppreciationProvider for CachedAppreciationProvider<P>
where
    P: AppreciationProvider + 'static,
{
    async fn appreciate(&self, request: AppreciationRequest) -> Result<Appreciation> {
        Ok(self.resolve(request).await?.appreciation)
    }

    async fn appreciate_stream(
        &self,
        request: AppreciationRequest,
    ) -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>> {
        if let Some(hit) = self.cache.lookup(&request, &self.provider.id())? {
            return Ok(start_operation(move |reporter| {
                reporter.item(AppreciationStreamItem::Complete(hit.appreciation));
                Ok(())
            }));
        }
        let inner = self.provider.appreciate_stream(request.clone()).await?;
        let cache = Arc::clone(&self.cache);
        Ok(start_operation(move |reporter| {
            loop {
                if reporter.is_cancelled() || reporter.is_closed() {
                    cancel(&inner);
                    return Ok(());
                }
                match next_event(&inner, 5) {
                    Some(Event::Progress(progress)) => {
                        reporter.progress(progress);
                    }
                    Some(Event::Item(AppreciationStreamItem::Chunk(chunk))) => {
                        if !reporter.item(AppreciationStreamItem::Chunk(chunk)) {
                            cancel(&inner);
                            return Ok(());
                        }
                    }
                    Some(Event::Item(AppreciationStreamItem::Complete(appreciation))) => {
                        cache
                            .store_completed(&request, &appreciation)
                            .map_err(|error| error.to_string())?;
                        if !reporter.item(AppreciationStreamItem::Complete(appreciation)) {
                            cancel(&inner);
                            return Ok(());
                        }
                    }
                    Some(Event::Done) => return Ok(()),
                    Some(Event::Cancelled) => {
                        cancel(&inner);
                        return Ok(());
                    }
                    Some(Event::Failed { message }) => return Err(message),
                    None => {}
                }
            }
        }))
    }

    fn id(&self) -> ProviderId {
        self.provider.id()
    }
}

fn cache_key(request: &AppreciationRequest, provider: &ProviderId) -> [u8; 32] {
    let mut hasher = Hasher::new();
    for component in [
        provider.as_str(),
        request.model(),
        request.style().unwrap_or(""),
        request.template_version(),
        &request.poem().poem.stable_id,
        request.grounding_digest(),
    ] {
        hasher.update(component.as_bytes());
        hasher.update(&[0]);
    }
    hasher.update(&request.temperature().to_bits().to_le_bytes());
    *hasher.finalize().as_bytes()
}

fn nonnegative_i64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn nonnegative_usize(value: i64, column: usize) -> rusqlite::Result<usize> {
    usize::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AppreciationCache, CacheSource, CachedAppreciationProvider, PurgeScope, ShippedAppreciation,
    };
    use crate::provider::tests::fixture_detail;
    use crate::{
        APPRECIATION_TEMPLATE, Appreciation, AppreciationProgress, AppreciationProvider,
        AppreciationRequest, AppreciationStreamItem, PromptTemplate, ProviderId,
    };
    use async_trait::async_trait;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use yunjian_core::operation::{OperationHandle, start_operation};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "yunjian-ai-cache-{label}-{}-{}",
                std::process::id(),
                line!()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("创建测试目录");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Clone)]
    struct CountingProvider {
        id: ProviderId,
        calls: Arc<AtomicUsize>,
    }

    impl CountingProvider {
        fn new(id: &str) -> Self {
            Self {
                id: ProviderId::new(id).expect("合法 provider"),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl AppreciationProvider for CountingProvider {
        async fn appreciate(
            &self,
            request: AppreciationRequest,
        ) -> yunjian_core::Result<Appreciation> {
            let sequence = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Appreciation {
                text: format!("第 {sequence} 次模型生成"),
                model: request.model().to_owned(),
                provider: self.id(),
                generated_at: sequence as u64,
                template_version: request.template_version().to_owned(),
                grounding_digest: request.grounding_digest().to_owned(),
                usage: None,
            })
        }

        async fn appreciate_stream(
            &self,
            request: AppreciationRequest,
        ) -> yunjian_core::Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>>
        {
            let result = self.appreciate(request).await?;
            Ok(start_operation(move |reporter| {
                reporter.item(AppreciationStreamItem::Complete(result));
                Ok(())
            }))
        }

        fn id(&self) -> ProviderId {
            self.id.clone()
        }
    }

    fn request() -> AppreciationRequest {
        AppreciationRequest::new(fixture_detail(), "user-model")
    }

    fn shipped(request: &AppreciationRequest, text: &str) -> ShippedAppreciation {
        ShippedAppreciation {
            stable_id: request.poem().poem.stable_id.clone(),
            template_version: request.template_version().to_owned(),
            model: "open-weight-model".to_owned(),
            model_license: "MIT".to_owned(),
            grounding_digest: request.grounding_digest().to_owned(),
            text: text.to_owned(),
            generated_at: 1,
        }
    }

    fn cache(directory: &TestDir, capacity: usize) -> Arc<AppreciationCache> {
        Arc::new(
            AppreciationCache::open(directory.path(), "corpus-v1", capacity).expect("打开赏析缓存"),
        )
    }

    #[tokio::test]
    async fn shipped_hit_is_provider_independent_and_performs_zero_provider_calls() {
        let directory = TestDir::new("shipped-hit");
        let cache = cache(&directory, 8);
        let request = request();
        cache
            .insert_shipped(&shipped(&request, "内置赏析"))
            .expect("写随包行");
        let provider = CountingProvider::new("different-user-provider");
        let cached = CachedAppreciationProvider::new(provider.clone(), Arc::clone(&cache));

        let result = cached.appreciate(request).await.expect("读取随包赏析");

        assert_eq!(result.text, "内置赏析");
        assert_eq!(provider.calls(), 0, "跨 provider 的随包命中不得调用模型");
    }

    #[tokio::test]
    async fn cold_poem_calls_once_then_uses_the_user_paid_cache() {
        let directory = TestDir::new("cold");
        let cache = cache(&directory, 8);
        let provider = CountingProvider::new("user-provider");
        let cached = CachedAppreciationProvider::new(provider.clone(), Arc::clone(&cache));
        let request = request();

        let first = cached.appreciate(request.clone()).await.expect("首次生成");
        let second = cached.appreciate(request).await.expect("二次读取缓存");

        assert_eq!(first, second);
        assert_eq!(provider.calls(), 1, "冷诗只应付费生成一次");
        assert_eq!(cache.counts().expect("统计").local, 1);
    }

    #[tokio::test]
    async fn template_version_bump_misses_both_tiers() {
        let directory = TestDir::new("template-version");
        let cache = cache(&directory, 8);
        let current = request();
        cache
            .insert_shipped(&shipped(&current, "旧模板内置赏析"))
            .expect("写随包行");
        let old_provider = CountingProvider::new("user-provider");
        cache
            .store_completed(
                &current,
                &old_provider
                    .appreciate(current.clone())
                    .await
                    .expect("造本地缓存"),
            )
            .expect("写本地缓存");
        let next_template = PromptTemplate::register(
            "appreciation",
            "appreciation.1.0.1.md",
            "1.0.1",
            APPRECIATION_TEMPLATE.source(),
        )
        .expect("注册新模板");
        let next = AppreciationRequest::with_template(
            current.poem().clone(),
            current.model(),
            next_template,
        );
        let provider = CountingProvider::new("user-provider");
        let cached = CachedAppreciationProvider::new(provider.clone(), Arc::clone(&cache));

        let result = cached.appreciate(next).await.expect("新模板生成");

        assert_eq!(result.text, "第 1 次模型生成");
        assert_eq!(provider.calls(), 1, "模板升级必须同时穿透两级缓存");
    }

    #[tokio::test]
    async fn changed_grounding_marks_shipped_stale_and_falls_through_but_metadata_only_edit_hits() {
        let directory = TestDir::new("grounding");
        let cache = cache(&directory, 8);
        let original = request();
        cache
            .insert_shipped(&shipped(&original, "按原文生成的内置赏析"))
            .expect("写随包行");

        let mut metadata_edit = original.poem().clone();
        metadata_edit.poem.content_hash = "corrected-content-hash".to_owned();
        let metadata_request = AppreciationRequest::new(metadata_edit, original.model());
        let metadata_hit = cache
            .lookup(
                &metadata_request,
                &ProviderId::new("other").expect("provider"),
            )
            .expect("查缓存")
            .expect("不改变 grounding 的修正仍应命中");
        assert_eq!(metadata_hit.source, CacheSource::Shipped);

        let mut text_edit = original.poem().clone();
        text_edit.poem.body.push_str("今已校正。");
        let changed = AppreciationRequest::new(text_edit, original.model());
        let provider = CountingProvider::new("user-provider");
        let cached = CachedAppreciationProvider::new(provider.clone(), Arc::clone(&cache));

        let result = cached.appreciate(changed).await.expect("陈旧后重新生成");

        assert_eq!(result.text, "第 1 次模型生成");
        assert_eq!(provider.calls(), 1);
        assert!(
            cache
                .shipped_is_stale(&original.poem().poem.stable_id, original.template_version())
                .expect("读 stale 标记"),
            "摘要不匹配必须保留并标陈旧"
        );
        assert_eq!(cache.counts().expect("统计").shipped, 1);
    }

    #[tokio::test]
    async fn lru_eviction_removes_only_user_paid_rows() {
        let directory = TestDir::new("lru");
        let cache = cache(&directory, 1);
        let first = request();
        cache
            .insert_shipped(&shipped(&first, "不可淘汰的内置赏析"))
            .expect("写随包行");
        let provider = CountingProvider::new("user-provider");
        let cached = CachedAppreciationProvider::new(provider.clone(), Arc::clone(&cache));
        cached
            .appreciate(first.clone())
            .await
            .expect("随包命中不写本地");
        let mut second_detail = first.poem().clone();
        second_detail.poem.stable_id = "poem-second".to_owned();
        second_detail.poem.body = "第二首冷诗。".to_owned();
        let second = AppreciationRequest::new(second_detail, first.model());
        let mut third_detail = first.poem().clone();
        third_detail.poem.stable_id = "poem-third".to_owned();
        third_detail.poem.body = "第三首冷诗。".to_owned();
        let third = AppreciationRequest::new(third_detail, first.model());
        cached.appreciate(second).await.expect("缓存第二首");
        std::thread::sleep(Duration::from_millis(2));
        cached.appreciate(third).await.expect("缓存第三首并淘汰");

        let counts = cache.counts().expect("统计");
        assert_eq!(counts.local, 1);
        assert_eq!(counts.shipped, 1, "LRU 永远不得删除随包行");
    }

    #[tokio::test]
    async fn cache_writes_never_modify_the_read_only_corpus_file() {
        let directory = TestDir::new("corpus-immutable");
        let corpus = directory.path().join("corpus.db");
        std::fs::write(&corpus, b"read-only-corpus-probe").expect("写语料探针");
        let before = std::fs::metadata(&corpus).expect("写前元数据");
        let cache = cache(&directory, 8);
        let provider = CountingProvider::new("user-provider");
        CachedAppreciationProvider::new(provider, Arc::clone(&cache))
            .appreciate(request())
            .await
            .expect("写赏析缓存");
        let after = std::fs::metadata(&corpus).expect("写后元数据");

        assert_eq!(before.len(), after.len());
        assert_eq!(
            before.modified().expect("写前 mtime"),
            after.modified().expect("写后 mtime"),
            "缓存只能写 app data SQLite，不能触碰语料库"
        );
    }

    #[tokio::test]
    async fn purge_scopes_delete_only_matching_user_paid_rows() {
        let directory = TestDir::new("purge");
        let cache = cache(&directory, 8);
        let first = request();
        cache
            .insert_shipped(&shipped(&first, "随包行"))
            .expect("写随包行");
        let provider = CountingProvider::new("user-provider");
        let first_generated = provider
            .appreciate(first.clone())
            .await
            .expect("生成第一行");
        cache
            .store_completed(&first, &first_generated)
            .expect("缓存第一行");

        let next_template = PromptTemplate::register(
            "appreciation",
            "appreciation.1.0.1.md",
            "1.0.1",
            APPRECIATION_TEMPLATE.source(),
        )
        .expect("注册新模板");
        let second =
            AppreciationRequest::with_template(first.poem().clone(), first.model(), next_template);
        let second_generated = provider
            .appreciate(second.clone())
            .await
            .expect("生成第二行");
        cache
            .store_completed(&second, &second_generated)
            .expect("缓存第二行");

        assert_eq!(
            cache
                .purge(PurgeScope::Template("1.0.0".to_owned()))
                .expect("按模板清理"),
            1
        );
        assert_eq!(cache.counts().expect("统计").local, 1);
        assert_eq!(
            cache
                .purge(PurgeScope::Poem(first.poem().poem.stable_id.clone()))
                .expect("按诗清理"),
            1
        );
        assert_eq!(cache.purge(PurgeScope::All).expect("全清"), 0);
        assert_eq!(cache.counts().expect("统计").shipped, 1);
    }
}
