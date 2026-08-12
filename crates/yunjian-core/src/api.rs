//! 稳定、可序列化且与宿主外壳无关的核心 API 门面。

use crate::{
    Attribution, AuthorDetail, CorpusHandle, MetaPage, PoemDetail, PoemFeatures, Result,
    RhymeAnswer, RhymeBook, RhymeGroupMatches, RhymeGroupRef, SearchPage, TagSummary,
    TextSearchRequest, ToneFilter, author_detail, browse_by_dynasty, browse_by_tag, do_these_rhyme,
    find_by_author, find_by_first_line, find_by_last_char, find_by_rhyme_group, find_by_title,
    find_work_group_attributions, frequent_content_chars, list_tags, poem_detail, poem_features,
    rhyme_groups_of,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 可廉价克隆并跨线程共享的云笺核心客户端。
#[derive(Debug, Clone)]
pub struct Yunjian {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    corpus: CorpusHandle,
}

macro_rules! paged_request {
    ($name:ident, $field:ident, $field_doc:literal, $type_doc:literal) => {
        #[doc = $type_doc]
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name {
            #[doc = $field_doc]
            pub $field: String,
            /// 上一页返回的续页游标。
            pub cursor: Option<String>,
        }
    };
}

paged_request!(
    TitleSearchRequest,
    query,
    "题目、词牌或合成题目的查询文本。",
    "按题目检索的请求。"
);
paged_request!(
    AuthorSearchRequest,
    query,
    "作者名或作者名前缀。",
    "按作者检索的请求。"
);
paged_request!(
    AuthorDetailRequest,
    query,
    "作者名或作者名前缀。",
    "读取作者详情的请求。"
);
paged_request!(
    DynastyBrowseRequest,
    dynasty,
    "朝代的规范键。",
    "按朝代浏览的请求。"
);
paged_request!(
    FirstLineSearchRequest,
    prefix,
    "首句前缀。",
    "按首句前缀检索的请求。"
);
paged_request!(
    LastCharacterSearchRequest,
    character,
    "单个句末字。",
    "按句末字检索的请求。"
);
paged_request!(
    TagBrowseRequest,
    tag,
    "构建期登记的策展标签。",
    "按策展标签浏览的请求。"
);

/// 读取作品分组全部归属的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkGroupRequest {
    /// 不含作者的作品分组键。
    pub work_group: String,
}

/// 按韵部检索作品的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RhymeGroupSearchRequest {
    /// 用于解释韵部名的韵书。
    pub book: RhymeBook,
    /// 韵部名，可带平水韵声部前缀。
    pub rhyme_group: String,
    /// 可选的声调筛选。
    pub tone: ToneFilter,
}

/// 判断多个字是否押韵的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RhymeCheckRequest {
    /// 待判断的字，至少两个。
    pub characters: Vec<char>,
    /// 判断所依据的韵书。
    pub book: RhymeBook,
}

/// 查询单字韵书归属的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterRhymesRequest {
    /// 待查询的字。
    pub character: char,
    /// 查询所依据的韵书。
    pub book: RhymeBook,
}

/// 读取作品详情的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoemDetailRequest {
    /// 作品的稳定标识。
    pub poem_id: String,
}

impl Yunjian {
    /// 从一份已就绪的只读语料库创建客户端。
    #[must_use]
    pub fn new(corpus: CorpusHandle) -> Self {
        Self {
            inner: Arc::new(Inner { corpus }),
        }
    }

    /// 检索正文或残句。
    pub fn search_text(&self, request: TextSearchRequest) -> Result<SearchPage> {
        self.inner.corpus.search_text(request)
    }

    /// 按题目、词牌或合成题目检索。
    pub fn find_by_title(&self, request: TitleSearchRequest) -> Result<MetaPage> {
        find_by_title(
            &self.inner.corpus,
            &request.query,
            request.cursor.as_deref(),
        )
    }

    /// 按作者名或作者名前缀检索。
    pub fn find_by_author(&self, request: AuthorSearchRequest) -> Result<MetaPage> {
        find_by_author(
            &self.inner.corpus,
            &request.query,
            request.cursor.as_deref(),
        )
    }

    /// 读取作者详情及归属冲突。
    pub fn author_detail(&self, request: AuthorDetailRequest) -> Result<AuthorDetail> {
        author_detail(
            &self.inner.corpus,
            &request.query,
            request.cursor.as_deref(),
        )
    }

    /// 按朝代规范键浏览。
    pub fn browse_by_dynasty(&self, request: DynastyBrowseRequest) -> Result<MetaPage> {
        browse_by_dynasty(
            &self.inner.corpus,
            &request.dynasty,
            request.cursor.as_deref(),
        )
    }

    /// 按首句前缀检索。
    pub fn find_by_first_line(&self, request: FirstLineSearchRequest) -> Result<MetaPage> {
        find_by_first_line(
            &self.inner.corpus,
            &request.prefix,
            request.cursor.as_deref(),
        )
    }

    /// 按句末字检索。
    pub fn find_by_last_character(&self, request: LastCharacterSearchRequest) -> Result<MetaPage> {
        find_by_last_char(
            &self.inner.corpus,
            &request.character,
            request.cursor.as_deref(),
        )
    }

    /// 读取同一作品分组里的全部归属。
    pub fn work_group_attributions(&self, request: WorkGroupRequest) -> Result<Vec<Attribution>> {
        find_work_group_attributions(&self.inner.corpus, &request.work_group)
    }

    /// 按韵书、韵部和声调检索作品。
    pub fn find_by_rhyme_group(
        &self,
        request: RhymeGroupSearchRequest,
    ) -> Result<RhymeGroupMatches> {
        find_by_rhyme_group(
            &self.inner.corpus,
            request.book,
            &request.rhyme_group,
            request.tone,
        )
    }

    /// 判断多个字在指定韵书里是否相押。
    pub fn do_these_rhyme(&self, request: RhymeCheckRequest) -> Result<RhymeAnswer> {
        do_these_rhyme(&self.inner.corpus, &request.characters, request.book)
    }

    /// 查询一个字在指定韵书里的全部归属。
    pub fn rhyme_groups_of(&self, request: CharacterRhymesRequest) -> Result<Vec<RhymeGroupRef>> {
        rhyme_groups_of(&self.inner.corpus, request.character, request.book)
    }

    /// 列出全部策展标签及作品数。
    pub fn list_tags(&self) -> Result<Vec<TagSummary>> {
        list_tags(&self.inner.corpus)
    }

    /// 按文档频率降序取前 `limit` 个正文字，供字面重叠计算排除常用字。
    pub fn frequent_content_chars(&self, limit: usize) -> Result<Vec<char>> {
        frequent_content_chars(&self.inner.corpus, limit)
    }

    /// 批量读取若干作品的本体、标签与韵部归属，供相关作品排序比对属性。
    pub fn poem_features(&self, poem_ids: &[&str]) -> Result<Vec<PoemFeatures>> {
        poem_features(&self.inner.corpus, poem_ids)
    }

    /// 按策展标签浏览作品。
    pub fn browse_by_tag(&self, request: TagBrowseRequest) -> Result<MetaPage> {
        browse_by_tag(&self.inner.corpus, &request.tag, request.cursor.as_deref())
    }

    /// 读取作品本体、格律、出处、标签和历代集评。
    pub fn poem_detail(&self, request: PoemDetailRequest) -> Result<PoemDetail> {
        poem_detail(&self.inner.corpus, &request.poem_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthorDetailRequest, AuthorSearchRequest, CharacterRhymesRequest, DynastyBrowseRequest,
        FirstLineSearchRequest, LastCharacterSearchRequest, PoemDetailRequest, RhymeCheckRequest,
        RhymeGroupSearchRequest, TagBrowseRequest, TitleSearchRequest, WorkGroupRequest, Yunjian,
    };
    use crate::{
        Attribution, AuthorDetail, CharacterRhymes, MetaPage, PoemDetail, Result, RhymeAnswer,
        RhymeGroupMatches, RhymeGroupRef, SearchPage, TagSummary, TextSearchRequest,
    };
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    fn assert_owned_api_type<T>()
    where
        T: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
    }

    #[test]
    fn facade_is_one_cheap_cloning_arc() {
        assert_eq!(
            std::mem::size_of::<Yunjian>(),
            std::mem::size_of::<std::sync::Arc<()>>()
        );
        fn assert_client<T: Clone + Send + Sync + 'static>() {}
        assert_client::<Yunjian>();
    }

    #[test]
    fn every_public_request_and_response_is_owned_serializable_send_sync_static() {
        assert_owned_api_type::<TextSearchRequest>();
        assert_owned_api_type::<TitleSearchRequest>();
        assert_owned_api_type::<AuthorSearchRequest>();
        assert_owned_api_type::<AuthorDetailRequest>();
        assert_owned_api_type::<DynastyBrowseRequest>();
        assert_owned_api_type::<FirstLineSearchRequest>();
        assert_owned_api_type::<LastCharacterSearchRequest>();
        assert_owned_api_type::<WorkGroupRequest>();
        assert_owned_api_type::<RhymeGroupSearchRequest>();
        assert_owned_api_type::<RhymeCheckRequest>();
        assert_owned_api_type::<CharacterRhymesRequest>();
        assert_owned_api_type::<TagBrowseRequest>();
        assert_owned_api_type::<PoemDetailRequest>();

        assert_owned_api_type::<SearchPage>();
        assert_owned_api_type::<MetaPage>();
        assert_owned_api_type::<AuthorDetail>();
        assert_owned_api_type::<Attribution>();
        assert_owned_api_type::<RhymeGroupMatches>();
        assert_owned_api_type::<RhymeAnswer>();
        assert_owned_api_type::<CharacterRhymes>();
        assert_owned_api_type::<RhymeGroupRef>();
        assert_owned_api_type::<TagSummary>();
        assert_owned_api_type::<PoemDetail>();
    }

    #[test]
    fn facade_exposes_every_search_path_with_typed_arguments() {
        let _: fn(&Yunjian, TextSearchRequest) -> Result<SearchPage> = Yunjian::search_text;
        let _: fn(&Yunjian, TitleSearchRequest) -> Result<MetaPage> = Yunjian::find_by_title;
        let _: fn(&Yunjian, AuthorSearchRequest) -> Result<MetaPage> = Yunjian::find_by_author;
        let _: fn(&Yunjian, AuthorDetailRequest) -> Result<AuthorDetail> = Yunjian::author_detail;
        let _: fn(&Yunjian, DynastyBrowseRequest) -> Result<MetaPage> = Yunjian::browse_by_dynasty;
        let _: fn(&Yunjian, FirstLineSearchRequest) -> Result<MetaPage> =
            Yunjian::find_by_first_line;
        let _: fn(&Yunjian, LastCharacterSearchRequest) -> Result<MetaPage> =
            Yunjian::find_by_last_character;
        let _: fn(&Yunjian, WorkGroupRequest) -> Result<Vec<Attribution>> =
            Yunjian::work_group_attributions;
        let _: fn(&Yunjian, RhymeGroupSearchRequest) -> Result<RhymeGroupMatches> =
            Yunjian::find_by_rhyme_group;
        let _: fn(&Yunjian, RhymeCheckRequest) -> Result<RhymeAnswer> = Yunjian::do_these_rhyme;
        let _: fn(&Yunjian, CharacterRhymesRequest) -> Result<Vec<RhymeGroupRef>> =
            Yunjian::rhyme_groups_of;
        let _: fn(&Yunjian) -> Result<Vec<TagSummary>> = Yunjian::list_tags;
        let _: fn(&Yunjian, TagBrowseRequest) -> Result<MetaPage> = Yunjian::browse_by_tag;
        let _: fn(&Yunjian, PoemDetailRequest) -> Result<PoemDetail> = Yunjian::poem_detail;
    }

    #[test]
    fn facade_source_contains_no_public_borrow_or_trait_object() {
        let source = include_str!("api.rs");
        for line in source.lines().map(str::trim) {
            if line.starts_with("pub struct ") || line.starts_with("pub enum ") {
                assert!(!line.contains("<'"), "公开类型不得带生命周期：{line}");
                assert!(
                    !line.contains("dyn "),
                    "公开类型不得带 trait object：{line}"
                );
            }
            if line.starts_with("pub ") {
                assert!(
                    !line.contains("&'"),
                    "公开 API 不得暴露引用生命周期：{line}"
                );
            }
        }
    }

    #[test]
    fn dependency_manifest_excludes_shell_and_rejected_search_engines() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("读取 yunjian-core Cargo.toml");
        let parsed: toml::Value = toml::from_str(&manifest).expect("解析 yunjian-core Cargo.toml");
        let dependencies = parsed
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .expect("Cargo.toml 应有 dependencies 表");
        for forbidden in ["tauri", "tantivy", "jieba-rs", "lindera", "opencc-rust"] {
            assert!(
                !dependencies.contains_key(forbidden),
                "yunjian-core 不得依赖 {forbidden}"
            );
        }
    }
}
