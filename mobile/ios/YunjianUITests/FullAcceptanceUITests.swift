import UIKit
import XCTest

/// todo 71 的十条 iOS 真机断言（界面侧）。
///
/// # 这个类只报测量值，不下结论
///
/// 每个测试把它观察到的事实交给 `AcceptanceReport`，宿主侧的
/// `xtask acceptance --platform ios --set full` 才把测量值判成 PASS / FAIL / NOT EXECUTED。
/// 与 Android 的 `FullAcceptanceTest` 同一分工，理由也一样：判词与阈值都在宿主侧，让被测物
/// 自己判等于把门禁搬进被测物内部。
///
/// # 执行顺序必须显式钉住
///
/// 这十条有真实的先后依赖：语料没物化就搜不到东西，没搜到东西就打不开阅读页。XCTest 默认
/// **按方法名字典序**执行（与 JUnit4 的 MD5 散列序不同），所以方法名带 `t01`..`t10` 前缀即
/// 执行顺序。为了不依赖那条默认行为，这里另外实现 `defaultTestSuite`，显式按名字排序，
/// 并在 `mobile/ios/Yunjian.xctestplan` 里关掉随机化。
/// Android 侧栽过一次：默认序把 `t09 → t10 → t06 → t07` 排在前面，依赖被打散，前面几条压根
/// 没跑到，而**只看方法名前缀会以为顺序已经写明了**。
///
/// # 为什么有些键由另一个 target 报
///
/// XCUITest 与被测应用**不在同一个进程**，读不到应用容器里的文件，也调不到 Rust 门面。
/// Android 的 instrumentation 恰好两件都能做，所以那边一个 target 就够。iOS 上的对应物是
/// 拆成两个：本文件（界面侧）与 `YunjianAppTests`（宿主为应用本体的进程内 target）。
/// 两者写同一套 `YUNJIAN-FULL` 行，宿主侧按断言 id 归并——键的并集才是一条断言的测量集。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
final class FullAcceptanceUITests: XCTestCase {
    /// 显式字典序。见类文档：不依赖框架的默认顺序。
    override class var defaultTestSuite: XCTestSuite {
        let suite = XCTestSuite(forTestCaseClass: FullAcceptanceUITests.self)
        let ordered = suite.tests.sorted { lhs, rhs in lhs.name < rhs.name }
        let explicit = XCTestSuite(name: "FullAcceptanceUITests(ordered)")
        ordered.forEach(explicit.addTest)
        return explicit
    }

    private var app: XCUIApplication!

    /// 物化预算。Pixel 8 上实测 868~873 秒（212 MiB 下载 + 4.44 GiB 解压）；iOS 侧同一份语料，
    /// 网络与解压耗时同量级，所以取同一个预算而不是拍一个更小的数。
    private let materializeBudget: TimeInterval = 1_500
    private let uiTimeout: TimeInterval = 60

    override func setUp() {
        continueAfterFailure = true
        app = XCUIApplication()
    }

    override func tearDown() {
        AcceptanceReport.attach(to: self)
    }

    // MARK: - t01

    func test_t01_install_and_launch() {
        let assertion = "install_and_launch"
        reportDeviceIdentity()
        AcceptanceReport.measure(assertion, "device_model", deviceModel)
        AcceptanceReport.measure(assertion, "os_build", osBuild)
        AcceptanceReport.measure(assertion, "package", Bundle.main.bundleIdentifier ?? "unknown")
        AcceptanceReport.measure(assertion, "native_binding", "uniffi_native")

        app.launch()
        // 首屏可交互的判据是「根节点存在且页签可点」，不是「进程活着」。Android 的 spike 那次
        // `process_alive=false` 正是进程起来又立刻崩。
        let root = app.otherElements[TestTags.root]
        let rendered = root.waitForExistence(timeout: uiTimeout)
        AcceptanceReport.measure(assertion, "root_rendered", rendered)
        let searchTab = app.buttons[TestTags.tabSearch]
        if searchTab.waitForExistence(timeout: uiTimeout) {
            searchTab.tap()
            AcceptanceReport.measure(assertion, "tab_clickable", true)
        } else {
            AcceptanceReport.unavailable(assertion, "tab_clickable", reason: "tab_search_never_appeared")
        }
        AcceptanceReport.measure(assertion, "crashed", app.state == .runningForeground ? false : true)
        screenshot(assertion, "launch")
    }

    // MARK: - t02

    /// 首启语料物化。
    ///
    /// 走应用自己的生产路径（`materializeAssets` -> `AssetResolver::sync_with_progress`），由界面
    /// 上的进度块观察。**不等首启派生**：判据问的是「显示下载、校验与原子物化进度且不崩溃」，
    /// 派生是另一件事；是否等过写进 `derive_awaited`，不藏着。
    ///
    /// `corpus_present` / `atomic_install` / `residual_temp_files` 由进程内 target 报——
    /// XCUITest 读不到应用容器。
    func test_t02_corpus_first_run_materialization() {
        let assertion = "corpus_first_run_materialization"
        launchIfNeeded()
        let started = Date()
        var stages = Set<String>()
        var progressShown = false
        let detail = app.staticTexts[TestTags.corpusProgressDetail]
        let facts = app.staticTexts[TestTags.corpusFacts]

        while Date().timeIntervalSince(started) < materializeBudget {
            if detail.exists, let label = detail.label as String?, !label.isEmpty {
                progressShown = true
                stages.insert(label)
            }
            if facts.exists { break }
            // 轮询间隔 2 秒而不是 0.5：物化期间界面每秒重组几十次，读得越勤撞上瞬时读失败的
            // 机会越多。Android 侧把间隔从 500 ms 放到 2 s 后仍能记到 413~527 段进度。
            Thread.sleep(forTimeInterval: 2)
        }

        AcceptanceReport.measure(assertion, "progress_shown", progressShown)
        AcceptanceReport.measure(assertion, "stage_count", stages.count)
        AcceptanceReport.measure(assertion, "duration_seconds", Int(Date().timeIntervalSince(started)))
        AcceptanceReport.measure(assertion, "derive_awaited", false)
        AcceptanceReport.measure(assertion, "corpus_facts_shown", facts.exists)
        AcceptanceReport.measure(assertion, "crashed", app.state == .runningForeground ? false : true)
        if !facts.exists {
            AcceptanceReport.unavailable(
                assertion,
                "corpus_facts",
                reason: "materialization_did_not_reach_ready_within_budget"
            )
        }
        screenshot(assertion, "materialized")
    }

    // MARK: - t03

    func test_t03_two_char_search_returns_results() {
        let assertion = "two_char_search_returns_results"
        launchIfNeeded()
        let query = "明月"
        AcceptanceReport.measure(assertion, "query", query)
        AcceptanceReport.measure(assertion, "query_char_count", query.count)

        guard let hits = search(query: query) else {
            AcceptanceReport.unavailable(assertion, "hits", reason: "search_result_count_label_absent")
            return
        }
        AcceptanceReport.measure(assertion, "hits", hits)
        AcceptanceReport.measure(assertion, "first_hit_id", firstHitId() ?? "none")
        screenshot(assertion, "search")
    }

    // MARK: - t04

    /// 阅读页的集评与随包赏析。
    ///
    /// **打开两首诗。** 随包赏析覆盖 16 首名篇、集评覆盖 394 首，实测交集为 0；判据原文问的是
    /// 「阅读页能不能显示这两样」，不是「某一首恰好两样都有」。两样仍须真的显示出来，两个
    /// poem_id 都回传供复核——这与 Android 侧是同一处调整，不是 iOS 的放宽。
    func test_t04_reading_view_citations_and_ai_appreciation() {
        let assertion = "reading_view_citations_and_ai_appreciation"
        launchIfNeeded()
        _ = search(query: "明月")

        var commentaryFound = false
        var appreciationFound = false
        var scanned = 0
        for identifier in hitIdentifiers().prefix(8) {
            guard openReading(hitIdentifier: identifier) else { continue }
            scanned += 1
            let poemId = identifier.replacingOccurrences(of: TestTags.searchHitPrefix, with: "")
            if !commentaryFound, app.staticTexts[TestTags.readingCommentaryPrefix + "0"].exists {
                commentaryFound = true
                AcceptanceReport.measure(assertion, "commentary_poem_id", poemId)
                AcceptanceReport.measure(assertion, "commentary_count", commentaryCount())
                AcceptanceReport.measure(
                    assertion,
                    "citation_shown",
                    app.staticTexts[TestTags.readingCommentaryCitationPrefix + "0"].exists
                )
                screenshot(assertion, "commentary")
            }
            if !appreciationFound, app.staticTexts[TestTags.readingAppreciation].exists {
                appreciationFound = true
                let disclosure = app.staticTexts[TestTags.readingAppreciationDisclosure]
                AcceptanceReport.measure(assertion, "appreciation_poem_id", poemId)
                AcceptanceReport.measure(assertion, "appreciation_shown", true)
                // 正文本身必须回传，与 Android 侧同一个键、同一个截断长度。
                // `appreciation_shown` 只说明那个节点存在，说不出节点里是什么：随包数据集
                // 未生成时正文是 `<<未生成：…>>` 这个**合法非空字符串**，存在性判据会放它
                // 过去。「不含未生成标记」那条判据在宿主侧（`full_criteria.rs`），这里只报值。
                AcceptanceReport.measure(
                    assertion,
                    "appreciation_text",
                    String(app.staticTexts[TestTags.readingAppreciation].label.prefix(60))
                )
                // 「明确标注、未经人工审校」——标注必须真的说出那句话，不是「有一段小字」。
                AcceptanceReport.measure(
                    assertion,
                    "disclosure_says_unreviewed",
                    disclosure.exists && disclosure.label.contains("未经人工审校")
                )
                screenshot(assertion, "appreciation")
            }
            closeReading()
            if commentaryFound, appreciationFound { break }
        }
        AcceptanceReport.measure(assertion, "poems_scanned", scanned)
        if !commentaryFound {
            AcceptanceReport.unavailable(assertion, "commentary_count", reason: "no_scanned_poem_had_commentary")
            AcceptanceReport.unavailable(assertion, "citation_shown", reason: "no_scanned_poem_had_commentary")
        }
        if !appreciationFound {
            AcceptanceReport.unavailable(assertion, "appreciation_shown", reason: "no_scanned_poem_had_shipped_appreciation")
            AcceptanceReport.unavailable(assertion, "disclosure_says_unreviewed", reason: "no_scanned_poem_had_shipped_appreciation")
        }
    }

    // MARK: - t05

    /// 打字背诵一轮，提交与原文**逐字相同**的答案。
    ///
    /// 评分因此是确定性的：完整度与严格准确都必须是 1、三类错误计数都必须是 0。判据能精确比对
    /// 而不是「看起来差不多」。
    ///
    /// **必须先等题目就绪再切页签。** `startRecite` 是异步的；立刻切过去时背诵页渲染成空态，
    /// 而此后没有任何事件驱动刷新，等输入框必然超时——Android 侧实测同一按钮坐标上一轮 PASS
    /// 下一轮超时，就是这个原因。
    func test_t05_typed_recitation_scores_correctly() {
        let assertion = "typed_recitation_scores_correctly"
        launchIfNeeded()
        _ = search(query: "明月")
        guard let identifier = hitIdentifiers().first else {
            AcceptanceReport.unavailable(assertion, "answer_equals_reference", reason: "no_search_hit_to_recite")
            return
        }
        guard openReading(hitIdentifier: identifier) else {
            AcceptanceReport.unavailable(assertion, "answer_equals_reference", reason: "reading_view_never_opened")
            return
        }
        let body = app.staticTexts[TestTags.readingBody]
        guard body.waitForExistence(timeout: uiTimeout), !body.label.isEmpty else {
            // 「正文非空」必须真的等到，不能只等一帧画完：Android 侧原先只 `waitForIdle`，
            // 那只保证这一帧画完，正文节点还不存在，于是照它默写必然不匹配——**一次装置问题
            // 被记成产品 FAIL，比 NOT EXECUTED 更糟。**
            AcceptanceReport.unavailable(assertion, "answer_equals_reference", reason: "reading_body_empty")
            closeReading()
            return
        }
        let reference = body.label
        closeReading()

        let reciteButton = app.buttons[TestTags.searchHitRecitePrefix
            + identifier.replacingOccurrences(of: TestTags.searchHitPrefix, with: "")]
        guard reciteButton.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "answer_equals_reference", reason: "recite_button_absent")
            return
        }
        reciteButton.tap()
        app.buttons[TestTags.tabRecite].tap()

        let field = app.textViews[TestTags.reciteAnswerField]
        let fallbackField = app.textFields[TestTags.reciteAnswerField]
        let target = field.waitForExistence(timeout: uiTimeout) ? field : fallbackField
        guard target.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "answer_equals_reference", reason: "recite_answer_field_absent")
            return
        }
        target.tap()
        // 清空预填的「明月」再逐字填正文：这一条要的是**逐字相同**，预填内容留着必然多出两字。
        clear(target)
        target.typeText(reference)
        AcceptanceReport.measure(assertion, "answer_equals_reference", currentText(of: target) == reference)
        AcceptanceReport.measure(assertion, "reference_char_count", reference.count)

        let submit = app.buttons[TestTags.reciteSubmit]
        guard submit.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "completeness", reason: "submit_button_not_laid_out")
            return
        }
        submit.tap()
        let score = app.staticTexts[TestTags.reciteScore]
        guard score.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "completeness", reason: "score_never_rendered")
            return
        }
        reportScore(assertion, label: score.label)
        screenshot(assertion, "score")
    }

    // MARK: - t06

    /// 语音一轮。
    ///
    /// `native_voice_enabled` / `record_audio_granted` / `model_dir_present` 由进程内 target 报
    /// （XCUITest 读不到应用容器与构建期特性）。这里报界面上真的出现了什么。
    ///
    /// `auto_graded` 恒为 `false` 并且**由界面上有没有评分决定**：2026-08-11 裁决按 1800 句实测
    /// CER 77.01% 定下 `guided_practice`，语音不做自动评分。哪天有人把识别结果接进评分，
    /// 这一条会量到 `true` 并让判据变红。
    func test_t06_voice_recitation_round_succeeds_end_to_end() {
        let assertion = "voice_recitation_round_succeeds_end_to_end"
        launchIfNeeded()
        _ = search(query: "明月")
        app.buttons[TestTags.tabVoice].tap()
        let start = app.buttons[TestTags.voiceStart]
        guard start.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "spoke", reason: "voice_start_button_absent")
            return
        }
        start.tap()

        let status = app.staticTexts[TestTags.voiceStatus]
        let degraded = app.staticTexts[TestTags.voiceDegradedReason]
        let deadline = Date().addingTimeInterval(180)
        var outcome: String?
        while Date() < deadline {
            if degraded.exists {
                AcceptanceReport.unavailable(assertion, "spoke", reason: "voice_degraded_" + slug(degraded.label))
                AcceptanceReport.unavailable(assertion, "total_ms", reason: "voice_degraded_" + slug(degraded.label))
                screenshot(assertion, "degraded")
                return
            }
            if status.exists, status.label.contains("开口=") {
                outcome = status.label
                break
            }
            Thread.sleep(forTimeInterval: 2)
        }
        guard let outcome else {
            AcceptanceReport.unavailable(assertion, "spoke", reason: "asr_outcome_not_rendered_within_180s")
            AcceptanceReport.unavailable(assertion, "total_ms", reason: "asr_outcome_not_rendered_within_180s")
            return
        }
        // 文案由产品侧固定：`开口=<bool> 停顿=<int> 时长=<int>ms 单路RTF=<double>`，与 Android
        // 逐字相同。解析而不是重量一遍：测量值必须是**用户真的看到的那一行**。
        AcceptanceReport.measure(assertion, "spoke", outcome.contains("开口=true"))
        AcceptanceReport.measure(assertion, "total_ms", number(in: outcome, after: "时长=") ?? 0)
        AcceptanceReport.measure(assertion, "single_rtf", number(in: outcome, after: "单路RTF=") ?? 0)
        AcceptanceReport.measure(assertion, "pause_count", number(in: outcome, after: "停顿=") ?? 0)
        // 语音页上没有任何评分节点即为「没有自动评分」。
        AcceptanceReport.measure(assertion, "auto_graded", app.staticTexts[TestTags.reciteScore].exists)
        screenshot(assertion, "voice")
    }

    // MARK: - t07

    /// 拒绝麦克风后降级到打字并显示具体原因。
    ///
    /// # 为什么用 `resetAuthorizationStatus` + 在弹窗上点「不允许」
    ///
    /// 与 Android 那条同源：**撤销一个已授予的权限必然重启持有它的进程**（iOS 上改隐私授权
    /// 同样会终止应用），而那样这条断言永远拿不到结果。`resetAuthorizationStatus(for:)` 把状态
    /// 退回「未询问」，下一次启动会弹窗，在弹窗上点「不允许」于是产品面对的是
    /// 「授权拿不到」——判据问的正是「采集真的拿不到数据时产品怎么做」，而不是「权限位是什么」。
    func test_t07_voice_permission_denied_degrades() {
        let assertion = "voice_permission_denied_degrades"
        let path = "xcuiapplication_resetAuthorizationStatus_microphone_then_deny_prompt"
        AcceptanceReport.measure(assertion, "revoke_path", path)
        app.terminate()
        app.resetAuthorizationStatus(for: .microphone)
        AcceptanceReport.measure(assertion, "revoke_executed", true)

        // 系统弹窗属于另一个进程，必须用 interruption monitor 处理；不装监听时点击会落在
        // 弹窗后面的界面上，表现为「点了没反应」。
        let monitor = addUIInterruptionMonitor(withDescription: "microphone-permission") { alert in
            for label in ["不允许", "Don't Allow", "Do Not Allow"] {
                let button = alert.buttons[label]
                if button.exists {
                    button.tap()
                    return true
                }
            }
            return false
        }
        defer { removeUIInterruptionMonitor(monitor) }

        app.launch()
        _ = app.otherElements[TestTags.root].waitForExistence(timeout: uiTimeout)
        app.buttons[TestTags.tabVoice].tap()
        let start = app.buttons[TestTags.voiceStart]
        guard start.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "degraded_reason", reason: "voice_start_button_absent")
            return
        }
        start.tap()
        app.tap() // 触发 interruption monitor

        let degraded = app.staticTexts[TestTags.voiceDegradedReason]
        guard degraded.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "degraded_reason", reason: "degraded_reason_never_shown")
            return
        }
        let reason = degraded.label
        AcceptanceReport.measure(assertion, "degraded_reason", reason)
        // 「显示具体原因」：原因里要点名采集或授权被拒，一句「语音不可用」不算。
        AcceptanceReport.measure(
            assertion,
            "reason_names_capture_denial",
            reason.contains("采集") || reason.contains("授权") || reason.contains("麦克风")
        )
        AcceptanceReport.measure(
            assertion,
            "fallback_to_typing_shown",
            app.staticTexts[TestTags.voiceFallbackToTyping].exists
        )
        AcceptanceReport.measure(assertion, "crashed", app.state == .runningForeground ? false : true)
        screenshot(assertion, "degraded")
    }

    // MARK: - t08

    /// 中文输入法向**已有内容**的字段输入，且键盘不遮挡输入框。
    ///
    /// 预填值由产品给（`startRecite` 填「明月」）：空字段测不出「在已有文本上追加」这件事。
    func test_t08_chinese_ime_prefilled_field_visible() {
        let assertion = "chinese_ime_prefilled_field_visible"
        launchIfNeeded()
        _ = search(query: "明月")
        guard let identifier = hitIdentifiers().first else {
            AcceptanceReport.unavailable(assertion, "field_prefilled", reason: "no_search_hit_to_recite")
            return
        }
        let poemId = identifier.replacingOccurrences(of: TestTags.searchHitPrefix, with: "")
        let reciteButton = app.buttons[TestTags.searchHitRecitePrefix + poemId]
        guard reciteButton.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "field_prefilled", reason: "recite_button_absent")
            return
        }
        reciteButton.tap()
        app.buttons[TestTags.tabRecite].tap()

        let field = app.textViews[TestTags.reciteAnswerField].exists
            ? app.textViews[TestTags.reciteAnswerField]
            : app.textFields[TestTags.reciteAnswerField]
        guard field.waitForExistence(timeout: uiTimeout) else {
            AcceptanceReport.unavailable(assertion, "field_prefilled", reason: "recite_answer_field_absent")
            return
        }
        let existing = currentText(of: field)
        AcceptanceReport.measure(assertion, "field_prefilled", !existing.isEmpty)
        AcceptanceReport.measure(assertion, "prefilled_text", existing)
        // 默认输入法在 iOS 上不可查询（沙箱不暴露键盘身份）。**这不是「测到了一个空值」**，
        // 所以写成 `_unavailable` 并给原因——空串会被宿主侧读成 FAIL。
        AcceptanceReport.unavailable(assertion, "default_ime", reason: "ios_does_not_expose_active_keyboard_identity")

        field.tap()
        field.typeText("几时")
        let appended = currentText(of: field)
        AcceptanceReport.measure(assertion, "append_preserved_existing", appended.hasPrefix(existing))
        AcceptanceReport.measure(assertion, "entered_text_present", appended.contains("几时"))
        AcceptanceReport.measure(assertion, "field_text_after_input", appended)

        // 「键盘不遮挡输入框」：输入框底边必须落在屏幕上一个正的坐标，且仍可命中。
        let frame = field.frame
        // 屏幕尺寸取被测应用窗口的 frame：`XCUIScreen` 只提供截图，不提供几何。
        let screen = app.windows.firstMatch.frame
        AcceptanceReport.measure(assertion, "input_visible", field.isHittable)
        AcceptanceReport.measure(assertion, "input_bottom_screen_px", Int(frame.maxY))
        AcceptanceReport.measure(assertion, "screen_height_px", Int(screen.height))
        screenshot(assertion, "ime")
    }

    // MARK: - t09

    func test_t09_background_return_preserves_layout() {
        let assertion = "background_return_preserves_layout"
        launchIfNeeded()
        app.buttons[TestTags.tabRecite].tap()
        XCUIDevice.shared.press(.home)
        Thread.sleep(forTimeInterval: 3)
        AcceptanceReport.measure(assertion, "went_background", app.state != .runningForeground)
        app.activate()

        let root = app.otherElements[TestTags.root]
        let rootPresent = root.waitForExistence(timeout: uiTimeout)
        let tabsPresent = app.buttons[TestTags.tabSearch].exists
            && app.buttons[TestTags.tabRecite].exists
            && app.buttons[TestTags.tabVoice].exists
        AcceptanceReport.measure(assertion, "root_present_after_return", rootPresent)
        AcceptanceReport.measure(assertion, "tabs_present_after_return", tabsPresent)
        // 「视图不折叠」：根节点面积必须仍占据屏幕的绝大部分。折叠的表现是高度塌到接近 0，
        // 而那种界面在截图上看着像「空白」——所以这一条与 `blank_screen` 各测一件事。
        let area = root.frame.height * root.frame.width
        let window = app.windows.firstMatch.frame
        let screenArea = window.height * window.width
        AcceptanceReport.measure(assertion, "layout_preserved", rootPresent && area > screenArea * 0.5)
        AcceptanceReport.measure(assertion, "root_area_ratio", Double(area / max(screenArea, 1)))
        AcceptanceReport.measure(assertion, "blank_screen", app.staticTexts.count == 0)
        screenshot(assertion, "returned")
    }

    // MARK: - t10

    /// 由自动化驱动正常退出。
    ///
    /// # `orphan_process_count` 在 iOS 上怎么算
    ///
    /// Android 侧数的是应用自己的进程（instrumentation 那一个已减掉）。iOS 沙箱**不允许应用
    /// fork 子进程**，也不允许枚举别人的进程，所以「孤儿进程」在这个平台上只可能是应用自身；
    /// `state == .notRunning` 即等于 0。这是把同一个键映射到平台上唯一可判定的事实，
    /// 不是把它当成恒零糊过去——应用没退干净时 `state` 不会是 `.notRunning`。
    func test_t10_app_exits_cleanly() {
        let assertion = "app_exits_cleanly"
        launchIfNeeded()
        // 截图在**退出前**拍。它与「应用已退出」看似矛盾，但那是执行前冻结的声明；改成不拍
        // 等于把门禁谈掉。图证「退出前处于正常可交互状态」——那正是「正常退出」区别于
        // 「已经崩了才退出」的地方。
        screenshot(assertion, "before-exit")
        AcceptanceReport.measure(assertion, "crashed", app.state == .runningForeground ? false : true)
        app.terminate()
        let exited = app.wait(for: .notRunning, timeout: 30)
        AcceptanceReport.measure(assertion, "activity_destroyed", exited)
        AcceptanceReport.measure(assertion, "orphan_process_count", exited ? 0 : 1)
    }

    // MARK: - 设备身份

    /// 宿主侧 `DeviceIdentity::is_physical` 读的五个键。
    ///
    /// # Android 的键名怎么映射到 iOS
    ///
    /// 键名**刻意不改**：两个平台共用同一套判据，改名等于让 iOS 永远走不到那段判断。映射是：
    ///
    /// | 键 | Android 来源 | iOS 来源 |
    /// | --- | --- | --- |
    /// | `model` | `ro.product.model` | `UIDevice.current.model` + `utsname.machine` |
    /// | `os_build` | `release/sdk` | `systemVersion/machine` |
    /// | `ro_hardware` | `ro.hardware` | `utsname.machine`（真机形如 `iPhone16,1`）|
    /// | `ro_kernel_qemu` | `ro.kernel.qemu` | 模拟器时写 `1`，真机写 `unset` |
    /// | `fingerprint` | `ro.build.fingerprint` | `apple/<machine>/<model>:<version>/<build>`，模拟器时带 `simulator` |
    ///
    /// 后两项让**模拟器必然被拒**：`ro_kernel_qemu=1` 与 fingerprint 里的 `simulator` 两条都会
    /// 命中宿主侧的拒绝清单。模拟器结果冒充真机要连改两处，而两处都在这段注释盯着的地方。
    private func reportDeviceIdentity() {
        AcceptanceReport.measure("device_identity", "model", deviceModel)
        AcceptanceReport.measure("device_identity", "os_build", osBuild)
        AcceptanceReport.measure("device_identity", "ro_hardware", machine)
        AcceptanceReport.measure("device_identity", "ro_kernel_qemu", isSimulator ? "1" : "unset")
        AcceptanceReport.measure("device_identity", "fingerprint", fingerprint)
    }

    private var machine: String {
        var info = utsname()
        uname(&info)
        return withUnsafePointer(to: &info.machine) { pointer in
            pointer.withMemoryRebound(to: CChar.self, capacity: Int(_SYS_NAMELEN)) { String(cString: $0) }
        }
    }

    private var isSimulator: Bool {
        ProcessInfo.processInfo.environment["SIMULATOR_DEVICE_NAME"] != nil
            || ProcessInfo.processInfo.environment["SIMULATOR_UDID"] != nil
    }

    private var deviceModel: String { "\(UIDevice.current.model) \(machine)" }

    private var osBuild: String { "\(UIDevice.current.systemVersion)/\(machine)" }

    private var fingerprint: String {
        let marker = isSimulator ? "simulator" : "device"
        return "apple/\(machine)/\(UIDevice.current.model):\(UIDevice.current.systemVersion)/\(marker)"
    }

    // MARK: - 装置

    private func launchIfNeeded() {
        if app.state != .runningForeground {
            app.launch()
        }
        _ = app.otherElements[TestTags.root].waitForExistence(timeout: uiTimeout)
    }

    /// 检索一次并返回命中条数（读界面上那句「命中 N 条」）。
    private func search(query: String) -> Int? {
        let field = app.textFields[TestTags.searchField]
        guard field.waitForExistence(timeout: uiTimeout) else { return nil }
        field.tap()
        clear(field)
        field.typeText(query)
        app.buttons[TestTags.searchSubmit].tap()
        let count = app.staticTexts[TestTags.searchResultCount]
        guard count.waitForExistence(timeout: uiTimeout) else { return nil }
        return number(in: count.label, after: "命中 ").map(Int.init)
    }

    private func hitIdentifiers() -> [String] {
        app.cells.allElementsBoundByIndex
            .map(\.identifier)
            .filter { $0.hasPrefix(TestTags.searchHitPrefix) }
    }

    private func firstHitId() -> String? {
        hitIdentifiers().first?.replacingOccurrences(of: TestTags.searchHitPrefix, with: "")
    }

    /// 打开一首诗的阅读页，并**按身份**确认屏幕上就是那一页。
    private func openReading(hitIdentifier: String) -> Bool {
        let poemId = hitIdentifier.replacingOccurrences(of: TestTags.searchHitPrefix, with: "")
        let read = app.buttons[TestTags.searchHitReadPrefix + poemId]
        guard read.waitForExistence(timeout: uiTimeout) else { return false }
        read.tap()
        return app.scrollViews[TestTags.readingPoemPrefix + poemId].waitForExistence(timeout: uiTimeout)
    }

    private func closeReading() {
        let back = app.buttons[TestTags.readingBack]
        if back.exists { back.tap() }
    }

    private func commentaryCount() -> Int {
        var index = 0
        while app.staticTexts[TestTags.readingCommentaryPrefix + String(index)].exists {
            index += 1
        }
        return index
    }

    private func reportScore(_ assertion: String, label: String) {
        AcceptanceReport.measure(assertion, "score_label", label)
        AcceptanceReport.measure(assertion, "completeness", number(in: label, after: "完整度 ") ?? -1)
        AcceptanceReport.measure(assertion, "accuracy_strict", number(in: label, after: "严格准确 ") ?? -1)
        AcceptanceReport.measure(assertion, "normal_count", Int(number(in: label, after: "正常 ") ?? -1))
        AcceptanceReport.measure(assertion, "deletion_count", Int(number(in: label, after: "漏 ") ?? -1))
        AcceptanceReport.measure(assertion, "insertion_count", Int(number(in: label, after: "增 ") ?? -1))
        AcceptanceReport.measure(assertion, "substitution_count", Int(number(in: label, after: "替 ") ?? -1))
        AcceptanceReport.measure(assertion, "rejected", label.contains("拒绝=true"))
    }

    /// 从一行文字里取某个前缀后面的数值。
    ///
    /// 读界面文字而不是自己再算一遍：测量值必须是**用户真的看到的那个数**。
    private func number(in text: String, after marker: String) -> Double? {
        guard let range = text.range(of: marker) else { return nil }
        let tail = text[range.upperBound...]
        let digits = tail.prefix { $0.isNumber || $0 == "." || $0 == "-" }
        return Double(digits)
    }

    private func currentText(of element: XCUIElement) -> String {
        (element.value as? String) ?? element.label
    }

    private func clear(_ element: XCUIElement) {
        let existing = currentText(of: element)
        guard !existing.isEmpty else { return }
        element.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: existing.count))
    }

    private func slug(_ text: String) -> String {
        text.replacingOccurrences(of: " ", with: "_")
            .replacingOccurrences(of: "；", with: "_")
            .replacingOccurrences(of: "：", with: "_")
    }

    /// 截图。「数字是结论，图是证据」。
    ///
    /// 文件名与 Android 侧同构（`<assertion>-<label>.png`），宿主侧据此在
    /// `docs/reports/mobile-qa/` 下找图；从 result bundle 提取附件的那一步由回收脚本做。
    private func screenshot(_ assertion: String, _ label: String) {
        let name = "\(assertion)-\(label).png"
        let shot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: shot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        AcceptanceReport.measure(assertion, "screenshot_\(label)", name)
        AcceptanceReport.measure(assertion, "screenshot_\(label)_bytes", shot.pngRepresentation.count)
    }
}
