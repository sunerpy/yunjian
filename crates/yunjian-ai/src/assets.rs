//! 语料与随包赏析种子的统一安装入口。

use crate::{
    APPRECIATION_DATABASE_FILE, APPRECIATION_TEMPLATE_VERSION, AppreciationCache,
    DEFAULT_APPRECIATION_CACHE_CAPACITY, ShippedSeedStatus,
};
use std::path::{Path, PathBuf};
use yunjian_core::assets::{AssetLocation, AssetResolver, AssetsManifest};
use yunjian_core::{CorpusConfig, CorpusHandle, Error, Result};

/// 完成统一同步后可供外壳呈现的状态。
#[derive(Debug)]
pub struct ShippedAssets {
    /// 已就绪的只读语料。
    pub corpus: CorpusHandle,
    /// 本次校验的统一资产清单。
    pub manifest: AssetsManifest,
    /// 已校验并在事务提交后发布的种子文件。
    pub seed_path: PathBuf,
    /// 数据库中已提交的种子版本与记录统计。
    pub seed: ShippedSeedStatus,
}

/// 按环境覆盖或默认发布地址同步语料与赏析种子。
pub fn sync_shipped_assets(
    corpus: CorpusConfig,
    app_data_dir: impl AsRef<Path>,
) -> Result<ShippedAssets> {
    let app_data_dir = app_data_dir.as_ref();
    let resolver = AssetResolver::discover(corpus, app_data_dir);
    sync_with_resolver(&resolver, app_data_dir)
}

/// 按显式清单来源同步语料与赏析种子。
pub fn sync_shipped_assets_from(
    manifest: impl Into<AssetLocation>,
    corpus: CorpusConfig,
    app_data_dir: impl AsRef<Path>,
) -> Result<ShippedAssets> {
    let app_data_dir = app_data_dir.as_ref();
    let resolver = AssetResolver::new(manifest, corpus, app_data_dir);
    sync_with_resolver(&resolver, app_data_dir)
}

/// 读取已落地语料和已提交种子的组合状态，不联网也不创建空缓存库。
pub fn shipped_assets_status(
    corpus: CorpusHandle,
    app_data_dir: impl AsRef<Path>,
) -> Result<(CorpusHandle, ShippedSeedStatus)> {
    let app_data_dir = app_data_dir.as_ref();
    let database = app_data_dir.join(APPRECIATION_DATABASE_FILE);
    if !database.is_file() {
        return Err(Error::Corpus(format!(
            "尚无随包赏析种子：{} 不存在；请运行 `yunjian corpus fetch`",
            database.display()
        )));
    }
    let cache = AppreciationCache::open(
        app_data_dir,
        corpus.meta().corpus_version.clone(),
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )?;
    let seed = cache.shipped_status()?.ok_or_else(|| {
        Error::Corpus("尚未导入随包赏析种子；请运行 `yunjian corpus fetch`".to_owned())
    })?;
    Ok((corpus, seed))
}

fn sync_with_resolver(resolver: &AssetResolver, app_data_dir: &Path) -> Result<ShippedAssets> {
    let synced = resolver.sync(|seed, manifest, corpus| {
        let cache = AppreciationCache::open(
            app_data_dir,
            corpus.meta().corpus_version.clone(),
            DEFAULT_APPRECIATION_CACHE_CAPACITY,
        )?;
        cache
            .replace_shipped_seed(seed.path(), manifest, APPRECIATION_TEMPLATE_VERSION)
            .map(|_| ())
    })?;
    let cache = AppreciationCache::open(
        app_data_dir,
        synced.corpus.meta().corpus_version.clone(),
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )?;
    let seed = cache
        .shipped_status()?
        .ok_or_else(|| Error::Corpus("赏析种子导入已完成但数据库没有版本元数据".to_owned()))?;
    Ok(ShippedAssets {
        corpus: synced.corpus,
        manifest: synced.manifest,
        seed_path: synced.seed_path,
        seed,
    })
}
