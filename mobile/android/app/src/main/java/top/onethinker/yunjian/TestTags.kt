package top.onethinker.yunjian

/**
 * `androidTest` 与生产界面之间的唯一契约。
 *
 * # 为什么把 tag 抽成常量
 *
 * 断言靠 tag 找控件。tag 写成字面量时，改界面的人看不到测试在用它，于是「改个名字」会让
 * 一条断言从 PASS 变成找不到节点——而那种失败读起来像功能坏了。放在这里，任何一次重命名
 * 都会同时出现在两侧的编译单元里。
 */
object TestTags {
    const val ROOT = "yunjian_root"
    const val TAB_SEARCH = "tab_search"
    const val TAB_RECITE = "tab_recite"
    const val TAB_VOICE = "tab_voice"

    const val CORPUS_PROGRESS = "corpus_progress"
    const val CORPUS_PROGRESS_DETAIL = "corpus_progress_detail"
    const val CORPUS_FACTS = "corpus_facts"

    const val SEARCH_FIELD = "search_field"
    const val SEARCH_SUBMIT = "search_submit"
    const val SEARCH_RESULTS = "search_results"
    const val SEARCH_RESULT_COUNT = "search_result_count"
    const val SEARCH_HIT_PREFIX = "search_hit_"

    /**
     * 命中卡片里那两个**真的带点击处理器**的按钮。
     *
     * [`SEARCH_HIT_PREFIX`] 标在卡片上，而卡片本身没有 `onClick`——第一轮真机实测里
     * 断言点了卡片，什么也没发生，阅读页永不出现，`t04` 以
     * `ComposeTimeoutException: Condition still not satisfied after 60000 ms` 失败。
     * 「点得到」与「点了有用」是两件事，所以按钮各有自己的 tag。
     */
    const val SEARCH_HIT_READ_PREFIX = "search_hit_read_"
    const val SEARCH_HIT_RECITE_PREFIX = "search_hit_recite_"

    /**
     * 直接按 `poem_id` 打开阅读页的输入与按钮。
     *
     * # 为什么需要它
     *
     * 「集评带出处」与「随包 AI 赏析」这两件事**覆盖的作品互不相交**：随包赏析只有
     * 16 首名篇，集评覆盖 394 首，实测交集为 0。靠检索「明月」拿第一条命中去验它们，
     * 命中的是一首两样都没有的词，于是那条断言只能报「这首没有集评」——而判据问的是
     * 产品能不能显示集评，不是某一首有没有。
     *
     * 所以给一条按 id 直达的入口，让断言各自定位到自己覆盖集里的作品。它同时是产品
     * 功能（用户可以粘贴一个 id 直达），不是仅为测试而存在的后门。
     */
    const val DIRECT_ID_FIELD = "direct_id_field"
    const val DIRECT_ID_OPEN = "direct_id_open"

    /**
     * 阅读页当前显示的是哪一首：tag 本身带 `poem_id`。
     *
     * # 为什么判身份而不判「标题变了」
     *
     * 「等标题变化」在两种情形下各错一次：上一首还开着时，只等「正文非空」会立刻交出
     * **上一首**的正文（t05 照它默写必然不匹配，一次装置问题被记成产品 FAIL）；
     * 而重开**同一首**时标题压根不会变（t06 在 t05 之后正是这样），等变化必然超时。
     *
     * tag 里带 id 让「屏幕上这一页是不是我要的那一页」成为可直接判定的事实，两种情形
     * 同一条判据。tag 不进界面文本，用户看不到。
     */
    const val READING_POEM_PREFIX = "reading_poem_"

    /** 从阅读页回到检索。阅读页独占一屏，没有它用户就出不来。 */
    const val READING_BACK = "reading_back"

    const val READING_TITLE = "reading_title"
    const val READING_BODY = "reading_body"
    const val READING_COMMENTARY_PREFIX = "reading_commentary_"
    const val READING_COMMENTARY_CITATION_PREFIX = "reading_commentary_citation_"
    const val READING_APPRECIATION = "reading_appreciation"
    const val READING_APPRECIATION_DISCLOSURE = "reading_appreciation_disclosure"

    /**
     * 背诵页在**没有题目**时说的那句话。
     *
     * 它必须有 tag：第十九轮真机上背诵页停在这句提示，而断言只读 `RECITE_PROMPT`
     * （无题目时不渲染），于是判词写成「背诵页与错误横幅都没有文本」——屏幕上明明有字。
     * **报不出界面正在说什么，等于没有判词。**
     */
    const val RECITE_EMPTY = "recite_empty"

    const val RECITE_PROMPT = "recite_prompt"
    const val RECITE_ANSWER_FIELD = "recite_answer_field"
    const val RECITE_SUBMIT = "recite_submit"
    const val RECITE_SCORE = "recite_score"

    const val VOICE_START = "voice_start"
    const val VOICE_STATUS = "voice_status"
    const val VOICE_DEGRADED_REASON = "voice_degraded_reason"
    const val VOICE_FALLBACK_TO_TYPING = "voice_fallback_to_typing"

    const val ERROR_BANNER = "error_banner"
}
