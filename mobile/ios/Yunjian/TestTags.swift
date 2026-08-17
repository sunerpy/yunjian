/// 界面节点的稳定标识。
///
/// # 为什么每个字面量都必须与 Android 侧逐字相同
///
/// `mobile/android/app/src/main/java/top/onethinker/yunjian/TestTags.kt` 是同一批断言在
/// Android 上的标识来源，而十条验收判据（`xtask/src/acceptance/mobile/full_criteria.rs`）
/// 两个平台共用**同一套**测量键。标识一旦分叉，两个平台就会在「同名断言」下量不同的东西，
/// 而报告里看不出这件事——它只会显示某个平台的某条断言 NOT EXECUTED。
///
/// 因此 `xtask` 里有一条断言逐字比对两份清单（`ios_project.rs` 的 tag 平价检查）：
/// 改这里而不改 Android，或反过来，都会变红。
///
/// # 与 Android 的机制差异（刻意保留）
///
/// Compose 用 `Modifier.testTag`，SwiftUI 用 `.accessibilityIdentifier`。前者只对测试可见，
/// 后者同时被辅助技术读到——所以 SwiftUI 侧凡是纯装置用途的标识都挂在**已有可见文本**的
/// 节点上，不新造只为测试存在的空节点。
enum TestTags {
    static let root = "yunjian_root"
    static let tabSearch = "tab_search"
    static let tabRecite = "tab_recite"
    static let tabVoice = "tab_voice"

    static let corpusProgress = "corpus_progress"
    static let corpusProgressDetail = "corpus_progress_detail"
    static let corpusFacts = "corpus_facts"

    static let searchField = "search_field"
    static let searchSubmit = "search_submit"
    static let searchResults = "search_results"
    static let searchResultCount = "search_result_count"
    static let searchHitPrefix = "search_hit_"
    static let searchHitReadPrefix = "search_hit_read_"
    static let searchHitRecitePrefix = "search_hit_recite_"

    static let directIdField = "direct_id_field"
    static let directIdOpen = "direct_id_open"
    static let readingPoemPrefix = "reading_poem_"
    static let readingBack = "reading_back"

    static let readingTitle = "reading_title"
    static let readingBody = "reading_body"
    static let readingCommentaryPrefix = "reading_commentary_"
    static let readingCommentaryCitationPrefix = "reading_commentary_citation_"
    static let readingAppreciation = "reading_appreciation"
    static let readingAppreciationDisclosure = "reading_appreciation_disclosure"

    static let reciteEmpty = "recite_empty"
    static let recitePrompt = "recite_prompt"
    static let reciteAnswerField = "recite_answer_field"
    static let reciteSubmit = "recite_submit"
    static let reciteScore = "recite_score"

    static let voiceStart = "voice_start"
    static let voiceStatus = "voice_status"
    static let voiceDegradedReason = "voice_degraded_reason"
    static let voiceFallbackToTyping = "voice_fallback_to_typing"

    static let errorBanner = "error_banner"
}
