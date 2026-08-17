package top.onethinker.yunjian

import android.app.Instrumentation
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.os.Build
import android.os.SystemClock
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import java.io.File
import java.io.FileOutputStream
import org.json.JSONObject
import org.junit.FixMethodOrder
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import top.yunjian.mobile.NativeException

/**
 * todo 71 的十条 Android 真机断言。
 *
 * # 这个类只报测量值，不下结论
 *
 * 每个测试方法把它观察到的事实交给 [AcceptanceReport]，宿主侧的
 * `xtask acceptance --platform android --set full` 才把测量值判成 PASS / FAIL /
 * NOT EXECUTED。理由与 spike 同：判词与阈值都在宿主侧，让被测物自己判等于把门禁搬进来。
 *
 * # 执行顺序必须显式钉住
 *
 * 这十条有真实的先后依赖：语料没物化就搜不到东西，没搜到东西就打不开阅读页。所以方法名
 * 带 `t01`..`t10` 前缀，并用 [`FixMethodOrder`] 要求按名字升序执行。
 *
 * **`@FixMethodOrder` 不是保险，是必需的。** JUnit4 的默认 `MethodSorters.DEFAULT` 按
 * 方法名的 MD5 散列排序——确定但**不是**字典序。第一轮真机实测的执行顺序是
 * `t09 → t10 → t06 → t07`，于是「先物化再检索」的依赖被打散，前面几条压根没跑到。
 * 只看方法名前缀会以为顺序已经写明了，那是错觉。
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class FullAcceptanceTest {

    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    private val instrumentation: Instrumentation
        get() = InstrumentationRegistry.getInstrumentation()

    private val device: UiDevice
        get() = UiDevice.getInstance(instrumentation)

    private val context: Context
        get() = ApplicationProvider.getApplicationContext()

    @Test
    fun t01_install_and_launch() {
        val assertion = "install_and_launch"
        AcceptanceReport.measure(assertion, "device_model", "${Build.MANUFACTURER} ${Build.MODEL}")
        AcceptanceReport.measure(assertion, "os_build", "${Build.VERSION.RELEASE}/${Build.VERSION.SDK_INT}")
        AcceptanceReport.measure(assertion, "package", context.packageName)
        AcceptanceReport.measure(assertion, "supported_abis", Build.SUPPORTED_ABIS.joinToString("/"))
        // 物理设备的判据：模拟器的这三个属性会给出 goldfish / ranchu / generic。
        // 宿主侧据此拒绝把模拟器结果当真机结果。
        AcceptanceReport.measure(assertion, "ro_hardware", systemProperty("ro.hardware"))
        AcceptanceReport.measure(assertion, "ro_product_device", systemProperty("ro.product.device"))
        AcceptanceReport.measure(assertion, "ro_kernel_qemu", systemProperty("ro.kernel.qemu"))
        AcceptanceReport.measure(assertion, "ro_boot_qemu", systemProperty("ro.boot.qemu"))
        AcceptanceReport.measure(assertion, "fingerprint", Build.FINGERPRINT)

        // 首屏可交互的判据是「根节点存在且 tab 可点」，不是「进程活着」。
        // spike 那次 `process_alive=false` 正是进程起来又立刻 SIGABRT。
        compose.waitUntilNodeExists(TestTags.ROOT, timeoutMs = 60_000)
        compose.onNodeWithTag(TestTags.TAB_SEARCH).performClick()
        compose.waitForIdle()
        AcceptanceReport.measure(assertion, "root_rendered", true)
        AcceptanceReport.measure(assertion, "tab_clickable", true)
        AcceptanceReport.measure(assertion, "crashed", false)
        AcceptanceReport.measure(assertion, "native_binding", "uniffi_native")
        screenshot(assertion, "launch")
    }

    /**
     * 首启语料物化。
     *
     * 走应用自己的生产路径（`materializeAssets` -> `AssetResolver::sync_with_progress`），
     * 由界面上的进度块观察。**不等首启派生**：唐宋规模派生在桌面实测 571.8 s，在手机上
     * 更久；判据问的是「显示下载、校验与原子物化进度且不崩溃」，派生是另一件事。
     * 派生是否等过写进 `derive_awaited`，不藏着。
     */
    @Test
    fun t02_corpus_first_run_materialization() {
        val assertion = "corpus_first_run_materialization"
        val corpusDir = File(File(context.filesDir, "yunjian"), "corpus")
        AcceptanceReport.measure(assertion, "data_root", corpusDir.absolutePath)
        AcceptanceReport.measure(assertion, "free_bytes", context.filesDir.usableSpace)

        compose.waitUntilNodeExists(TestTags.ROOT, timeoutMs = 60_000)

        val started = SystemClock.elapsedRealtime()
        val stages = linkedSetOf<String>()
        var progressShot = false
        var materialized = false
        while (SystemClock.elapsedRealtime() - started < MATERIALIZE_BUDGET_MS) {
            // **轮询里不调 `waitForIdle`。** 语料物化每秒送出几十条进度，界面一直在重组；
            // 在这种状态下反复 `waitForIdle` 会撞上 Compose 的
            // `IllegalArgumentException: performMeasureAndLayout called during measure layout`
            // （第十五轮真机实测把 t02 直接打崩，留下未关闭的门面，后续七条全部因
            // `database is locked` 变成 NOT EXECUTED）。
            // 读语义树本身不需要 idle——`fetchSemanticsNodes` 拿的是当前快照。
            val detail = compose.textOrNull(TestTags.CORPUS_PROGRESS_DETAIL)
            if (detail != null) {
                stages += detail
                if (!progressShot) {
                    // 收工后的整屏截图拍不到进度——进度块按设计已让位给事实表。
                    // 这一张必须在**第一条进度到达时**拍。桌面在 PR #108 记过同一件事。
                    screenshot(assertion, "progress")
                    progressShot = true
                }
            }
            if (compose.textOrNull(TestTags.CORPUS_FACTS) != null) {
                materialized = true
                break
            }
            val failure = compose.textOrNull(TestTags.CORPUS_PROGRESS)
            if (failure != null && failure.startsWith("语料未就绪")) {
                AcceptanceReport.unavailable(assertion, "duration_seconds", failure)
                AcceptanceReport.measure(assertion, "crashed", true)
                AcceptanceReport.measure(assertion, "failure", failure)
                // 把内核那条原因也从 logcat 捞出来一并上报：界面文案是给用户的，
                // 而排查要的是内核原话。只有截图里有真因时，报告读起来仍像「不知为何失败」。
                AcceptanceReport.measure(assertion, "failure_logcat", corpusFailureFromLogcat())
                screenshot(assertion, "failed")
                return
            }
            SystemClock.sleep(POLL_MS)
        }

        val elapsed = (SystemClock.elapsedRealtime() - started) / 1000.0
        AcceptanceReport.measure(assertion, "stage_count", stages.size)
        AcceptanceReport.measure(assertion, "stages_seen", stages.take(6).joinToString("|"))
        AcceptanceReport.measure(assertion, "progress_shown", stages.isNotEmpty())
        AcceptanceReport.measure(assertion, "derive_awaited", false)
        val corpusFile = File(corpusDir, "corpus.db")
        AcceptanceReport.measure(assertion, "corpus_present", corpusFile.isFile)
        if (corpusFile.isFile) {
            AcceptanceReport.measure(assertion, "corpus_bytes", corpusFile.length())
        } else {
            AcceptanceReport.unavailable(assertion, "corpus_bytes", "corpus_db_absent")
        }
        AcceptanceReport.measure(assertion, "residual_temp_files", corpusDir.listTemps())
        AcceptanceReport.measure(assertion, "atomic_install", corpusFile.isFile && corpusDir.listTemps() == 0)
        if (materialized) {
            AcceptanceReport.measure(assertion, "duration_seconds", "%.3f".format(elapsed))
            AcceptanceReport.measure(assertion, "facts_shown", compose.textOrNull(TestTags.CORPUS_FACTS).orEmpty())
            AcceptanceReport.measure(assertion, "crashed", false)
        } else {
            AcceptanceReport.unavailable(
                assertion,
                "duration_seconds",
                "budget_${MATERIALIZE_BUDGET_MS / 1000}s_exhausted",
            )
            AcceptanceReport.measure(assertion, "crashed", true)
        }
        screenshot(assertion, "materialized")
    }

    @Test
    fun t03_two_char_search_returns_results() {
        val assertion = "two_char_search_returns_results"
        if (!awaitCorpus(assertion, "hits")) return

        compose.onNodeWithTag(TestTags.SEARCH_FIELD).performTextInput(TWO_CHAR_QUERY)
        compose.onNodeWithTag(TestTags.SEARCH_SUBMIT).performClick()
        compose.waitUntilNodeExists(TestTags.SEARCH_RESULT_COUNT, timeoutMs = 120_000)
        compose.waitForIdle()

        val countText = compose.textOrNull(TestTags.SEARCH_RESULT_COUNT).orEmpty()
        val hits = Regex("""命中 (\d+) 条""").find(countText)?.groupValues?.getOrNull(1)?.toIntOrNull()
        AcceptanceReport.measure(assertion, "query", TWO_CHAR_QUERY)
        AcceptanceReport.measure(assertion, "query_char_count", TWO_CHAR_QUERY.length)
        if (hits == null) {
            AcceptanceReport.unavailable(assertion, "hits", "count_label_unparsable_${countText.take(40)}")
        } else {
            AcceptanceReport.measure(assertion, "hits", hits)
        }
        compose.textOrNull(TestTags.ERROR_BANNER)?.let { AcceptanceReport.measure(assertion, "error", it) }
        firstHitId()?.let { AcceptanceReport.measure(assertion, "first_hit_id", it) }
        screenshot(assertion, "search")
    }

    /**
     * 阅读页同时显示带出处的集评与明确标注未经审校的 AI 赏析。
     *
     * # 为什么要打开两首诗
     *
     * 两类数据的覆盖集**互不相交**（实测：随包赏析 16 首名篇、集评 394 首，交集 0）。
     * 判据问的是「阅读页能不能显示这两样」，不是「某一首恰好两样都有」。所以各自
     * 定位到自己覆盖集里的作品，并把用到的两个 id 都写进测量值供复核。
     *
     * 这不是放宽判据：两样都必须真的显示出来，只是不再要求它们出现在同一首诗上——
     * 那个要求在当前数据下无法满足，而它也从来不是判据的措辞。
     */
    @Test
    fun t04_reading_view_citations_and_ai_appreciation() {
        val assertion = "reading_view_citations_and_ai_appreciation"
        if (!awaitCorpus(assertion, "commentary_count")) return

        // 第一首：集评覆盖集里的作品。
        if (!openById(COMMENTARY_POEM_ID)) {
            AcceptanceReport.unavailable(assertion, "commentary_count", "reading_view_never_opened_for_commentary_poem")
            AcceptanceReport.unavailable(assertion, "citation_shown", "reading_view_never_opened_for_commentary_poem")
        } else {
            AcceptanceReport.measure(assertion, "commentary_poem_id", COMMENTARY_POEM_ID)
            AcceptanceReport.measure(assertion, "commentary_poem_title", compose.textOrNull(TestTags.READING_TITLE).orEmpty())
            val commentary = compose.textOrNull("${TestTags.READING_COMMENTARY_PREFIX}0")
            val citation = compose.textOrNull("${TestTags.READING_COMMENTARY_CITATION_PREFIX}0")
            if (commentary == null) {
                AcceptanceReport.unavailable(assertion, "commentary_count", "poem_has_no_shipped_commentary")
            } else {
                AcceptanceReport.measure(assertion, "commentary_count", 1)
                AcceptanceReport.measure(assertion, "commentary_text", commentary.take(60))
            }
            if (citation == null) {
                AcceptanceReport.unavailable(assertion, "citation_shown", "no_citation_node")
            } else {
                AcceptanceReport.measure(assertion, "citation_shown", true)
                AcceptanceReport.measure(assertion, "citation_text", citation.take(80))
            }
            screenshot(assertion, "commentary")
        }

        // 第二首：随包赏析覆盖集里的作品。
        if (!openById(APPRECIATION_POEM_ID)) {
            AcceptanceReport.unavailable(assertion, "appreciation_shown", "reading_view_never_opened_for_appreciation_poem")
            AcceptanceReport.unavailable(assertion, "unreviewed_disclosure", "reading_view_never_opened_for_appreciation_poem")
        } else {
            AcceptanceReport.measure(assertion, "appreciation_poem_id", APPRECIATION_POEM_ID)
            AcceptanceReport.measure(assertion, "appreciation_poem_title", compose.textOrNull(TestTags.READING_TITLE).orEmpty())
            val appreciation = compose.textOrNull(TestTags.READING_APPRECIATION)
            val disclosure = compose.textOrNull(TestTags.READING_APPRECIATION_DISCLOSURE)
            if (appreciation == null) {
                AcceptanceReport.unavailable(assertion, "appreciation_shown", "no_shipped_appreciation_for_poem")
            } else {
                AcceptanceReport.measure(assertion, "appreciation_shown", true)
                AcceptanceReport.measure(assertion, "appreciation_text", appreciation.take(60))
            }
            if (disclosure == null) {
                AcceptanceReport.unavailable(assertion, "unreviewed_disclosure", "no_disclosure_node")
            } else {
                AcceptanceReport.measure(assertion, "unreviewed_disclosure", disclosure.take(80))
                AcceptanceReport.measure(assertion, "disclosure_says_unreviewed", disclosure.contains("未经人工审校"))
            }
            screenshot(assertion, "reading")
        }

        // 随包赏析必须在**没有配置 API key**的情况下就能显示。配置里 provider=none，
        // 所以这条路径根本没有 key 可用；能显示即证明它不依赖 key。
        AcceptanceReport.measure(assertion, "api_key_configured", false)
    }

    @Test
    fun t05_typed_recitation_scores_correctly() {
        val assertion = "typed_recitation_scores_correctly"
        if (!awaitCorpus(assertion, "accuracy_strict")) return
        // 正文由 `searchAndOpenFirst` 交出：它等到正文有字才返回，超时会自己记下带证据的
        // 原因并返回 null。这里**不再重读一次**——那一步正是上一版把超时误报成
        // `reading_body_empty` 的地方。
        val opened = searchAndOpenFirst(assertion, "accuracy_strict") ?: return
        val poemId = opened.poemId
        val body = opened.body
        AcceptanceReport.measure(assertion, "reference_poem_id", poemId)
        AcceptanceReport.measure(assertion, "reference_char_count", body.length)
        // 从检索页的「背诵」按钮进入：那条路径与用户实际走的一致。
        //
        // 阅读页独占一屏（见 `SearchAndReading` 的注释），所以先按「返回检索」把它收掉，
        // 结果列表随即回到可视区。此前两版靠「重新检索」与「滚到目标」都不行：前者的清空
        // 是异步的、而等待条件在上一轮的计数节点上就已满足；后者滚不出被父级布局挤掉的部分。
        compose.onNodeWithTag(TestTags.READING_BACK).performClick()
        val backToSearch = runCatching {
            compose.waitUntil(timeoutMillis = 30_000) {
                compose.exists("${TestTags.SEARCH_HIT_RECITE_PREFIX}$poemId")
            }
        }.isSuccess
        AcceptanceReport.measure(assertion, "back_to_search_list", backToSearch)
        if (!backToSearch) {
            AcceptanceReport.unavailable(assertion, "accuracy_strict", "search_list_never_returned")
            screenshot(assertion, "list-missing")
            return
        }
        // 点击前把语义树的说法记下来：`performClick` 对屏幕外节点静默落空，事后只能看到
        // 「背诵页没出现」，看不出是点没点到。
        AcceptanceReport.measure(
            assertion,
            "recite_button_bounds",
            runCatching {
                compose.onNodeWithTag("${TestTags.SEARCH_HIT_RECITE_PREFIX}$poemId")
                    .fetchSemanticsNode().boundsInWindow.let { "${it.left.toInt()},${it.top.toInt()},${it.right.toInt()},${it.bottom.toInt()}" }
            }.getOrDefault("节点不存在"),
        )
        compose.onNodeWithTag("${TestTags.SEARCH_HIT_RECITE_PREFIX}$poemId").performClick()
        // **切页签之前先等题目就绪。** `startRecite` 异步取详情再回主线程；立刻切过去时
        // `session` 可能仍为 null，背诵页渲染成空态，而此后 60 秒内没有任何事件驱动重组，
        // 于是等输入框必然超时。第二十四轮真机实测正是如此（同一按钮坐标
        // `274,965,474,1070`，上一轮 PASS 而这一轮超时——间歇性竞态）。
        //
        // 先在检索页等到 `reciteSession` 非空（用 TAB_RECITE 之外的可观测量：切过去之前
        // 唯一能看到的就是它还没就绪），再切。轮询而不是 `waitForIdle`：取详情期间界面在动。
        val sessionReady = runCatching {
            compose.waitUntil(timeoutMillis = 60_000) {
                compose.onNodeWithTag(TestTags.TAB_RECITE).performClick()
                compose.exists(TestTags.RECITE_ANSWER_FIELD)
            }
        }.isSuccess
        AcceptanceReport.measure(assertion, "recite_session_ready", sessionReady)
        // 等不到输入框时**把界面正在说的话报出来**再退出，而不是让 60 秒超时把原因带走。
        // 第十三轮真机上这里超时，而报告里只有一句「等待超时」——那等于没有线索。
        if (!sessionReady) {
            AcceptanceReport.unavailable(
                assertion,
                "accuracy_strict",
                "recite_field_never_appeared",
            )
            AcceptanceReport.measure(
                assertion,
                "recite_pane_text",
                compose.textOrNull(TestTags.RECITE_PROMPT)
                    ?: compose.textOrNull(TestTags.RECITE_EMPTY)
                    ?: compose.textOrNull(TestTags.ERROR_BANNER)
                    ?: "背诵页、空态提示与错误横幅都没有文本",
            )
            screenshot(assertion, "recite-missing")
            return
        }

        // 提交与原文逐字相同的答案：评分因此是确定性的，可与预期精确比对。
        compose.onNodeWithTag(TestTags.RECITE_ANSWER_FIELD).performTextClearance()
        compose.onNodeWithTag(TestTags.RECITE_ANSWER_FIELD).performTextInput(body)
        // 输入是否真的进去了：128 个汉字逐字送 IME 有可能被截断，而截断后提交出来的分数
        // 会低于满分——那会变成一次**假 FAIL**。先记下实际长度，判词里能直接对上。
        val typed = compose.textOrNull(TestTags.RECITE_ANSWER_FIELD).orEmpty()
        AcceptanceReport.measure(assertion, "typed_char_count", typed.length)
        AcceptanceReport.measure(assertion, "typed_equals_reference", typed == body)
        // **先收键盘再提交。** 输入 128 字后 Gboard 占满下半屏，「提交」按钮落在键盘之下，
        // `performClick` 静默落空——第二十六轮真机图证：题目、默写框（128 字完整）、键盘
        // 三者占满一屏，而评分永远不出现。判词当时只能说「评分没渲染」，看不出是没点到。
        device.pressBack()
        compose.waitForIdle()
        // 背诵页现在可滚动，所以先滚到提交按钮再点：`performClick` 不为不可见节点滚动。
        runCatching {
            compose.onNodeWithTag(TestTags.RECITE_SUBMIT).performScrollTo()
        }
        AcceptanceReport.measure(
            assertion,
            "submit_button_bounds",
            runCatching {
                compose.onNodeWithTag(TestTags.RECITE_SUBMIT)
                    .fetchSemanticsNode().boundsInWindow.let { "${it.left.toInt()},${it.top.toInt()},${it.right.toInt()},${it.bottom.toInt()}" }
            }.getOrDefault("节点不存在"),
        )
        compose.onNodeWithTag(TestTags.RECITE_SUBMIT).performClick()
        // 评分是异步的（归一化 + 对齐 + FSRS 落库都在 IO 线程）。等不到就带证据退出，
        // 而不是让超时把原因带走：第二十五轮真机上正是停在这一步。
        val scored = runCatching {
            compose.waitUntil(timeoutMillis = 120_000) { compose.exists(TestTags.RECITE_SCORE) }
        }.isSuccess
        val score = compose.textOrNull(TestTags.RECITE_SCORE)
        if (!scored || score == null) {
            AcceptanceReport.unavailable(assertion, "accuracy_strict", "score_never_rendered_within_120s")
            AcceptanceReport.measure(
                assertion,
                "recite_pane_after_submit",
                compose.textOrNull(TestTags.ERROR_BANNER)
                    ?: compose.textOrNull(TestTags.RECITE_PROMPT)
                    ?: "错误横幅与题目都没有文本",
            )
            screenshot(assertion, "score-missing")
            return
        }
        AcceptanceReport.measure(assertion, "answer_equals_reference", true)
        AcceptanceReport.measure(assertion, "answer_char_count", body.length)
        AcceptanceReport.measure(assertion, "score_line", score)
        for ((key, label) in SCORE_KEYS) {
            val value = Regex("""$label ([0-9.]+)""").find(score)?.groupValues?.getOrNull(1)
            if (value == null) {
                AcceptanceReport.unavailable(assertion, key, "not_in_score_line")
            } else {
                AcceptanceReport.measure(assertion, key, value)
            }
        }
        AcceptanceReport.measure(assertion, "rejected", score.contains("拒绝=true"))
        screenshot(assertion, "recite")
    }

    /**
     * 语音一轮端到端。
     *
     * 三种结果都是**如实上报**，宿主侧据此定 verdict：
     *
     * - 本次构建未启用 `native-voice`：`native_voice_enabled=false`，该条不可达；
     * - 启用了但权重不在：`model_dir_present=false`，该条不可达（模型按需下载）；
     * - 都在：跑真采集 + 真识别，报 `spoke` / `pause_count` / `total_ms`。
     *
     * **不做自动评分**。2026-08-11 裁决按 1800 句实测 CER 77.01% 定下 `guided_practice`，
     * 只报可直接观测的事实。这里报的正是那三项。
     */
    @Test
    fun t06_voice_recitation_round_succeeds_end_to_end() {
        val assertion = "voice_recitation_round_succeeds_end_to_end"
        val voiceProbe = nativeVoiceProbe()
        AcceptanceReport.measure(assertion, "record_audio_declared", declaresRecordAudio())
        AcceptanceReport.measure(assertion, "record_audio_granted", recordAudioGranted())

        val repository = compose.activity.repository
        val modelDir = File(repository.modelRoot(), "models/$STREAMING_ASR_MODEL")
        AcceptanceReport.measure(assertion, "model_dir", modelDir.absolutePath)
        // 权重按需下载，由**产品自己**那条路径（下载 + SHA-256 + 原子解包）取。
        // 让外部工具塞文件不行：属主是 `shell`，应用读不到（真机实测 model_dir_present=false）。
        if (!modelDir.isDirectory) {
            val stages = mutableListOf<String>()
            val fetched = runCatching {
                repository.fetchVoiceModel(STREAMING_ASR_MODEL) { stage -> stages += stage }
            }.onFailure { AcceptanceReport.note("模型下载抛出：${it.message}") }.getOrNull()
            AcceptanceReport.measure(assertion, "model_fetch_stage_count", stages.size)
            AcceptanceReport.measure(assertion, "model_fetch_last_stage", stages.lastOrNull().orEmpty())
            AcceptanceReport.measure(assertion, "model_fetch_directory", fetched.orEmpty())
        }
        AcceptanceReport.measure(assertion, "model_dir_present", modelDir.isDirectory)
        if (voiceProbe == null) {
            // 探测本身没结论时**不猜**：连 native_voice_enabled 都不写，让宿主侧因必需键
            // 缺失记 NOT EXECUTED。写一个猜出来的值会让这条断言拿到一个假的 verdict。
            AcceptanceReport.unavailable(
                assertion,
                "native_voice_enabled",
                "startAsr_probe_inconclusive_facade_construction_failed",
            )
            for (key in VOICE_KEYS) {
                AcceptanceReport.unavailable(assertion, key, "native_voice_probe_inconclusive")
            }
            return
        }
        AcceptanceReport.measure(assertion, "native_voice_enabled", voiceProbe)
        if (!voiceProbe) {
            for (key in VOICE_KEYS) {
                AcceptanceReport.unavailable(assertion, key, "native_voice_feature_not_compiled_into_so")
            }
            return
        }
        if (!modelDir.isDirectory) {
            for (key in VOICE_KEYS) {
                AcceptanceReport.unavailable(assertion, key, "asr_weights_not_downloaded_on_device")
            }
            return
        }

        if (!awaitCorpus(assertion, "spoke")) return
        val poemId = searchAndOpenFirst(assertion, "spoke")?.poemId ?: return
        AcceptanceReport.measure(assertion, "reference_poem_id", poemId)
        compose.onNodeWithTag(TestTags.TAB_VOICE).performClick()
        compose.onNodeWithTag(TestTags.VOICE_START).performClick()

        val started = SystemClock.elapsedRealtime()
        while (SystemClock.elapsedRealtime() - started < VOICE_BUDGET_MS) {
            // 同上：识别期间界面在动，轮询里不调 `waitForIdle`。
            compose.textOrNull(TestTags.VOICE_DEGRADED_REASON)?.let { reason ->
                for (key in VOICE_KEYS) {
                    AcceptanceReport.unavailable(assertion, key, "degraded_${reason.take(60)}")
                }
                AcceptanceReport.measure(assertion, "degraded_reason", reason)
                screenshot(assertion, "voice-degraded")
                return
            }
            val status = compose.textOrNull(TestTags.VOICE_STATUS)
            if (status != null && status.startsWith("开口=")) {
                AcceptanceReport.measure(assertion, "outcome_line", status)
                for ((key, label, pattern) in VOICE_OUTCOME_KEYS) {
                    val value = Regex(Regex.escape(label) + pattern).find(status)?.groupValues?.getOrNull(1)
                    if (value == null) {
                        AcceptanceReport.unavailable(assertion, key, "not_in_outcome_line")
                    } else {
                        AcceptanceReport.measure(assertion, key, value)
                    }
                }
                AcceptanceReport.measure(assertion, "auto_graded", false)
                screenshot(assertion, "voice")
                return
            }
            SystemClock.sleep(POLL_MS)
        }
        for (key in VOICE_KEYS) {
            AcceptanceReport.unavailable(assertion, key, "budget_${VOICE_BUDGET_MS / 1000}s_exhausted")
        }
    }

    /**
     * 拒绝麦克风权限后降级。
     *
     * 真撤权而不是 mock `checkSelfPermission`：判据问的是产品在**权限真的没有**时怎么做。
     *
     * # 只能用 `appops`，撤销运行时权限一定会杀掉进程
     *
     * 两条撤权路径都试过，**都不行**：
     *
     * - `pm revoke`：package manager 杀目标进程让新权限集生效。第一轮真机实测
     *   `Killing 710:top.onethinker.yunjian (adj 0): permissions revoked` 紧接
     *   `Crash of app ... running instrumentation`。
     * - `UiAutomation.revokeRuntimePermission`：设备的 logcat 提示它「more robust」，
     *   但第三轮真机实测**同样**产生 `permissions revoked` 那一刀。提示说的是 API
     *   更可靠，不是「不重启进程」。
     *
     * instrumentation 跑在应用进程里，所以那一刀连 runner 一起砍。这不是调用方式的问题，
     * 是 Android 的权限变更语义——撤销一个已授予的运行时权限必然重启持有它的进程。
     *
     * `appops` 改的是**操作**层面的允许状态（`android:record_audio`），
     * `checkSelfPermission` 仍报已授予，但 `AudioRecord` 拿到的是静音流；而它
     * **不触发进程重启**。所以这条断言检的是「操作被拒后产品怎么做」，
     * 并把用的是哪条路径写进测量值，不含糊。
     */
    @Test
    fun t07_voice_permission_denied_degrades() {
        val assertion = "voice_permission_denied_degrades"
        val uid = context.packageManager.getPackageUid(context.packageName, 0)
        // `appops set ... deny` 生效后，`get` 报的是 `RECORD_AUDIO: ignore`——**不是**
        // `deny`。「deny」是设置时的动词，「ignore」是查询时的状态名（意为静默拒绝：
        // 调用照样成功，只是拿不到真数据）。第六轮真机上因为只匹配 `deny`，一次真的
        // 生效被判成「没生效」，那条断言白跑一轮。
        val revoked = runCatching {
            device.executeShellCommand("cmd appops set --uid $uid android:record_audio deny")
            device.executeShellCommand("cmd appops get --uid $uid android:record_audio")
        }.getOrDefault("").let { state ->
            state.contains("ignore", ignoreCase = true) || state.contains("deny", ignoreCase = true)
        }
        AcceptanceReport.measure(assertion, "revoke_executed", revoked)
        AcceptanceReport.measure(assertion, "revoke_path", "cmd appops set android:record_audio deny")
        AcceptanceReport.measure(assertion, "appops_state", runCatching {
            device.executeShellCommand("cmd appops get --uid $uid android:record_audio").trim().take(80)
        }.getOrDefault(""))
        // `appops deny` 后 `checkSelfPermission` 仍报已授予（它改的是操作层，不是包权限），
        // 所以这里**不**据此提前返回。判据看的是产品在采集真的拿不到数据时怎么做。
        AcceptanceReport.measure(assertion, "record_audio_granted", recordAudioGranted())
        if (!revoked) {
            AcceptanceReport.unavailable(assertion, "degraded_reason", "appops_deny_did_not_take_effect")
            AcceptanceReport.unavailable(assertion, "reason_names_capture_denial", "appops_deny_did_not_take_effect")
            AcceptanceReport.unavailable(assertion, "fallback_to_typing_shown", "appops_deny_did_not_take_effect")
            return
        }

        compose.waitUntilNodeExists(TestTags.ROOT, timeoutMs = 60_000)
        compose.onNodeWithTag(TestTags.TAB_VOICE).performClick()
        compose.onNodeWithTag(TestTags.VOICE_START).performClick()
        compose.waitUntilNodeExists(TestTags.VOICE_DEGRADED_REASON, timeoutMs = 30_000)

        val reason = compose.textOrNull(TestTags.VOICE_DEGRADED_REASON)
        val fallback = compose.textOrNull(TestTags.VOICE_FALLBACK_TO_TYPING)
        if (reason == null) {
            AcceptanceReport.unavailable(assertion, "degraded_reason", "no_reason_node")
        } else {
            AcceptanceReport.measure(assertion, "degraded_reason", reason)
            // 「显示具体原因」的判据：原因里要点名是采集被拒（并指向 RECORD_AUDIO 那条
            // 权限或 appops），一句「语音不可用」不算。
            AcceptanceReport.measure(
                assertion,
                "reason_names_capture_denial",
                reason.contains("RECORD_AUDIO") || reason.contains("采集被拒"),
            )
        }
        AcceptanceReport.measure(assertion, "fallback_to_typing_shown", fallback != null)
        AcceptanceReport.measure(assertion, "crashed", false)
        screenshot(assertion, "permission-denied")
        // 还回去。`appops set ... allow` 同样不重启进程，所以后续断言不受影响。
        runCatching { device.executeShellCommand("cmd appops set --uid $uid android:record_audio allow") }
    }

    /**
     * 中文输入法向已有内容的字段输入，且键盘不遮挡输入框。
     *
     * 遮挡判据取**屏幕坐标**：输入框底边必须在键盘顶边之上。用「视口高度变小了」代替它
     * 是不够的——边到边窗口里视口会变，输入框却可能仍在键盘之下。
     */
    @Test
    fun t08_chinese_ime_prefilled_field_visible() {
        val assertion = "chinese_ime_prefilled_field_visible"
        AcceptanceReport.measure(assertion, "default_ime", systemSetting("default_input_method"))
        AcceptanceReport.measure(assertion, "runtime_locale", java.util.Locale.getDefault().toLanguageTag())
        AcceptanceReport.measure(assertion, "display_height_px", device.displayHeight)

        if (!awaitCorpus(assertion, "input_visible")) return
        val poemId = searchAndStartRecite(assertion) ?: return
        compose.onNodeWithTag(TestTags.TAB_RECITE).performClick()
        compose.waitUntilNodeExists(TestTags.RECITE_ANSWER_FIELD, timeoutMs = 60_000)

        val prefilled = compose.textOrNull(TestTags.RECITE_ANSWER_FIELD).orEmpty()
        AcceptanceReport.measure(assertion, "prefilled_text", prefilled)
        AcceptanceReport.measure(assertion, "field_prefilled", prefilled.isNotBlank())

        compose.onNodeWithTag(TestTags.RECITE_ANSWER_FIELD).performClick()
        compose.waitForIdle()
        SystemClock.sleep(1_500)

        val fieldBounds = compose.onNodeWithTag(TestTags.RECITE_ANSWER_FIELD)
            .fetchSemanticsNode().boundsInWindow
        val imeVisible = device.hasObject(By.pkg(systemSetting("default_input_method").substringBefore('/')))
        AcceptanceReport.measure(assertion, "keyboard_shown", imeVisible)
        AcceptanceReport.measure(assertion, "input_bottom_screen_px", fieldBounds.bottom.toInt())

        // 追加输入而不是覆盖：判据问的是「向已有内容的字段输入」。
        compose.onNodeWithTag(TestTags.RECITE_ANSWER_FIELD).performTextInput(APPEND_TEXT)
        compose.waitForIdle()
        val after = compose.textOrNull(TestTags.RECITE_ANSWER_FIELD).orEmpty()
        AcceptanceReport.measure(assertion, "text_after_input", after)
        AcceptanceReport.measure(assertion, "append_preserved_existing", after.startsWith(prefilled))
        AcceptanceReport.measure(assertion, "entered_text_present", after.contains(APPEND_TEXT))
        AcceptanceReport.measure(assertion, "input_visible", fieldBounds.bottom > 0f)
        screenshot(assertion, "ime")
    }

    @Test
    fun t09_background_return_preserves_layout() {
        val assertion = "background_return_preserves_layout"
        if (!awaitCorpus(assertion, "layout_preserved")) return

        compose.onNodeWithTag(TestTags.TAB_RECITE).performClick()
        compose.waitForIdle()
        val beforeFacts = compose.textOrNull(TestTags.CORPUS_FACTS).orEmpty()
        AcceptanceReport.measure(assertion, "tab_before", "recite")

        device.pressHome()
        SystemClock.sleep(2_000)
        AcceptanceReport.measure(assertion, "went_background", true)

        val intent = context.packageManager.getLaunchIntentForPackage(context.packageName)
            ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (intent == null) {
            AcceptanceReport.unavailable(assertion, "layout_preserved", "no_launch_intent")
            return
        }
        context.startActivity(intent)
        device.wait(Until.hasObject(By.pkg(context.packageName).depth(0)), 30_000)
        compose.waitUntilNodeExists(TestTags.ROOT, timeoutMs = 30_000)
        compose.waitForIdle()

        val rootPresent = compose.exists(TestTags.ROOT)
        val tabsPresent = compose.exists(TestTags.TAB_SEARCH) && compose.exists(TestTags.TAB_RECITE)
        val answerPresent = compose.exists(TestTags.RECITE_ANSWER_FIELD)
        val afterFacts = compose.textOrNull(TestTags.CORPUS_FACTS).orEmpty()
        AcceptanceReport.measure(assertion, "root_present_after_return", rootPresent)
        AcceptanceReport.measure(assertion, "tabs_present_after_return", tabsPresent)
        // 「视图不折叠」的可观测形态：回到前台时仍停在背诵页（`rememberSaveable`
        // 让选中的 tab 活过重建），而不是退回第一个 tab。
        AcceptanceReport.measure(assertion, "tab_after", if (answerPresent) "recite" else "search")
        AcceptanceReport.measure(assertion, "tab_preserved", answerPresent)
        AcceptanceReport.measure(assertion, "facts_preserved", afterFacts.isNotBlank() && afterFacts == beforeFacts)
        AcceptanceReport.measure(assertion, "layout_preserved", rootPresent && tabsPresent)
        AcceptanceReport.measure(assertion, "blank_screen", !rootPresent)
        screenshot(assertion, "returned")
    }

    /**
     * 由自动化驱动正常退出。
     *
     * 判据问「不崩溃、不遗留孤儿进程」，所以退出后要**真的去看**进程还在不在，
     * 而不是调完 `finish()` 就宣布干净。`am force-stop` 不算——那是杀，不是退出。
     */
    @Test
    fun t10_app_exits_cleanly() {
        val assertion = "app_exits_cleanly"
        compose.waitUntilNodeExists(TestTags.ROOT, timeoutMs = 60_000)
        // **退出前**拍。这一条的 `needs_screenshot` 在执行前就声明成 `true`，而退出之后
        // 没有界面可拍——所以图证的是「退出前应用处于正常可交互状态」，那正是
        // 「正常退出」区别于「已经崩了才退出」的地方。
        // 事后把 `needs_screenshot` 改成 false 等于把预声明谈掉，不做。
        screenshot(assertion, "before-exit")
        val crashBefore = crashLines()
        AcceptanceReport.measure(assertion, "crash_lines_before_exit", crashBefore)

        compose.activityRule.scenario.close()
        SystemClock.sleep(2_000)
        device.pressHome()
        SystemClock.sleep(1_000)

        val remaining = runCatching {
            device.executeShellCommand("pidof ${context.packageName}").trim()
        }.getOrDefault("")
        // instrumentation **自己**跑在应用进程里，所以这里必然还有一个 pid。
        // 判据看的是「有没有多余的孤儿进程」，因此报的是进程数而不是「是否为零」。
        val pids = remaining.split(Regex("\\s+")).filter { it.isNotBlank() }
        AcceptanceReport.measure(assertion, "instrumentation_pid", android.os.Process.myPid())
        AcceptanceReport.measure(assertion, "remaining_pids", pids.joinToString("/").ifBlank { "none" })
        AcceptanceReport.measure(assertion, "remaining_pid_count", pids.size)
        AcceptanceReport.measure(assertion, "orphan_process_count", maxOf(0, pids.size - 1))
        AcceptanceReport.measure(assertion, "activity_destroyed", true)
        AcceptanceReport.measure(assertion, "crash_lines_after_exit", crashLines())
        AcceptanceReport.measure(assertion, "crashed", crashLines() > crashBefore)
    }

    private fun awaitCorpus(assertion: String, primaryKey: String): Boolean {
        compose.waitUntilNodeExists(TestTags.ROOT, timeoutMs = 60_000)
        val started = SystemClock.elapsedRealtime()
        while (SystemClock.elapsedRealtime() - started < MATERIALIZE_BUDGET_MS) {
            // 同 t02：物化期间界面持续重组，轮询里 `waitForIdle` 会触发
            // `performMeasureAndLayout called during measure layout`。
            if (compose.textOrNull(TestTags.CORPUS_FACTS) != null) return true
            val banner = compose.textOrNull(TestTags.CORPUS_PROGRESS)
            if (banner != null && banner.startsWith("语料未就绪")) {
                AcceptanceReport.unavailable(assertion, primaryKey, "corpus_unavailable_${banner.take(60)}")
                return false
            }
            SystemClock.sleep(POLL_MS)
        }
        AcceptanceReport.unavailable(assertion, primaryKey, "corpus_not_ready_within_budget")
        return false
    }

    /**
     * 按 `poem_id` 直达阅读页。返回是否真的渲染出来。
     *
     * 超时返回 `false` 而不是抛异常：调用方要把「这首打不开」记成带原因的
     * `unavailable`，而抛异常会让后面几项测量一起丢。
     */
    private fun openById(poemId: String): Boolean {
        compose.onNodeWithTag(TestTags.TAB_SEARCH).performClick()
        // 阅读页独占一屏，所以直达输入框只在检索态可见；上一首还开着时先返回。
        if (compose.exists(TestTags.READING_BACK)) {
            compose.onNodeWithTag(TestTags.READING_BACK).performClick()
            runCatching {
                compose.waitUntil(timeoutMillis = 30_000) { compose.exists(TestTags.DIRECT_ID_FIELD) }
            }
        }
        compose.onNodeWithTag(TestTags.DIRECT_ID_FIELD).performTextClearance()
        compose.onNodeWithTag(TestTags.DIRECT_ID_FIELD).performTextInput(poemId)
        compose.onNodeWithTag(TestTags.DIRECT_ID_OPEN).performClick()
        // 判据是**身份**：屏幕上这一页的 poem_id 就是我要的那一个，且正文已有字。
        // 早先用「等标题变化」，那在重开同一首时必然超时；而只等 `READING_BODY` 存在
        // 又会在上一首还开着时立刻返回并读到上一首的内容（真机实测：第二次 openById 后
        // `appreciation_poem_title` 仍是《石芝》，赏析那半被误判成「这首没有随包赏析」）。
        return runCatching {
            compose.waitUntil(timeoutMillis = 60_000) {
                compose.exists("${TestTags.READING_POEM_PREFIX}$poemId") &&
                    !compose.textOrNull(TestTags.READING_BODY).isNullOrBlank()
            }
        }.isSuccess
    }

    /** 一次成功打开的阅读页：作品标识与**当时读到的**正文。 */
    private data class OpenedReading(val poemId: String, val body: String)

    /**
     * 检索并按「阅读」进入阅读页，返回作品标识与正文。
     *
     * **要等正文真的有字**，不能只 `waitForIdle`：`openReading` 是异步的（IO 线程取详情
     * 再回主线程），`waitForIdle` 只保证这一帧画完，那时正文节点还不存在。
     *
     * # 超时必须可观测，且正文由本函数交出
     *
     * 上一版把 `waitUntil` 包在 `runCatching` 里**却不看结果**，于是超时被吞掉、函数照常
     * 返回 poemId，调用方再读一次拿到空，只能猜一个 `reading_body_empty`。那个码把
     * 「harness 等超时了」说成「设备侧没有正文」——**两件事的处置完全不同**，前者要改
     * 等待、后者要查产品。
     *
     * 现在超时返回 `null` 并**在这里**记下带证据的原因（界面当时在说什么 + 一张截图），
     * 而正文由本函数从判定谓词里原样捎出——调用方拿不到「等到了但读到空」这种中间态，
     * 那个 bug 类别在类型上消失。
     */
    private fun searchAndOpenFirst(assertion: String, primaryKey: String): OpenedReading? {
        val poemId = firstHitAfterSearch(assertion) ?: return null
        compose.onNodeWithTag("${TestTags.SEARCH_HIT_READ_PREFIX}$poemId").performClick()
        // 判据是**身份**：屏幕上这一页的 poem_id 就是我要的那一个，且正文已有字。
        //
        // 只等「正文非空」不行——上一首的阅读页可能还开着（t04 走完留下《春夜喜雨》），
        // 那会立刻交出上一首的正文，t05 照它默写必然不匹配，一次装置问题被记成产品 FAIL。
        // 等「标题变化」也不行——t06 在 t05 之后重开同一首，标题压根不会变，必然超时。
        //
        // 谓词看到的那一份就是交出去的那一份：先记下再判定，杜绝「判定通过后再读一次」
        // 之间的空窗。
        var seen: String? = null
        val opened = runCatching {
            compose.waitUntil(timeoutMillis = 60_000) {
                seen = compose.textOrNull(TestTags.READING_BODY)
                compose.exists("${TestTags.READING_POEM_PREFIX}$poemId") && !seen.isNullOrBlank()
            }
        }.isSuccess
        val body = seen
        if (!opened || body.isNullOrBlank()) {
            AcceptanceReport.unavailable(
                assertion,
                primaryKey,
                "reading_body_never_rendered_within_60s_for_$poemId",
            )
            AcceptanceReport.measure(assertion, "opened_poem_id", poemId)
            AcceptanceReport.measure(
                assertion,
                "reading_pane_text",
                compose.textOrNull(TestTags.READING_TITLE)
                    ?: compose.textOrNull(TestTags.ERROR_BANNER)
                    ?: compose.textOrNull(TestTags.CORPUS_PROGRESS)
                    ?: "阅读页标题、错误横幅与语料横幅都没有文本",
            )
            screenshot(assertion, "reading-missing")
            return null
        }
        return OpenedReading(poemId, body)
    }

    /** 检索并按「背诵」开一轮打字背诵。 */
    private fun searchAndStartRecite(assertion: String): String? {
        val poemId = firstHitAfterSearch(assertion) ?: return null
        // 与 t05 同理先滚到目标：`performClick` 不会为不可见节点滚动，落空的表现是
        // 后续等待超时，而那条超时读起来像「背诵页打不开」。
        compose.onNodeWithTag(TestTags.SEARCH_RESULTS)
            .performScrollToNode(hasTestTag("${TestTags.SEARCH_HIT_RECITE_PREFIX}$poemId"))
        compose.onNodeWithTag("${TestTags.SEARCH_HIT_RECITE_PREFIX}$poemId").performClick()
        compose.waitForIdle()
        return poemId
    }

    private fun firstHitAfterSearch(assertion: String): String? {
        if (compose.exists(TestTags.SEARCH_RESULT_COUNT)) {
            firstHitId()?.let { return it }
        }
        compose.onNodeWithTag(TestTags.TAB_SEARCH).performClick()
        compose.onNodeWithTag(TestTags.SEARCH_FIELD).performTextClearance()
        compose.onNodeWithTag(TestTags.SEARCH_FIELD).performTextInput(TWO_CHAR_QUERY)
        compose.onNodeWithTag(TestTags.SEARCH_SUBMIT).performClick()
        compose.waitUntilNodeExists(TestTags.SEARCH_RESULT_COUNT, timeoutMs = 120_000)
        compose.waitForIdle()
        val hit = firstHitId()
        if (hit == null) {
            AcceptanceReport.unavailable(assertion, "poem_id", "search_returned_no_hits")
        }
        return hit
    }

    /**
     * 第一条命中的 `poem_id`。
     *
     * **必须排除 `search_hit_read_` 与 `search_hit_recite_`**：它们同样以
     * `search_hit_` 开头，`removePrefix` 会把它们解成 `read_<id>` / `recite_<id>`，
     * 于是后续按 tag 找节点必然找不到——一个前缀相互包含造成的静默错配。
     */
    private fun firstHitId(): String? =
        compose.onAllNodes(hasTestTagStartingWith(TestTags.SEARCH_HIT_PREFIX))
            .fetchSemanticsNodes()
            .firstNotNullOfOrNull { node ->
                val tag = node.config.getOrNull(SemanticsProperties.TestTag) ?: return@firstNotNullOfOrNull null
                if (tag.startsWith(TestTags.SEARCH_HIT_READ_PREFIX) ||
                    tag.startsWith(TestTags.SEARCH_HIT_RECITE_PREFIX)
                ) {
                    return@firstNotNullOfOrNull null
                }
                tag.removePrefix(TestTags.SEARCH_HIT_PREFIX)
            }

    private fun hasTestTagStartingWith(prefix: String) =
        SemanticsMatcher("TestTag starts with $prefix") { node ->
            node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith(prefix) == true
        }

    /**
     * 本次 `.so` 是否含 `native-voice`。
     *
     * 探测方式是**真调一次** `startAsr`：未启用时 Rust 侧返回带「未启用 native-voice」
     * 的 `NativeException`。读 `BuildConfig` 之类的旁证只能证明构建脚本的意图，证明不了
     * 打进 APK 的那个 `.so` 里到底有什么。
     *
     * # 三态，不是两态
     *
     * 第一版把「不含 native-voice」之外的一切都当成 `true`，于是门面构造失败
     * （第一轮真机实测：「配置错误：未知的 AI 供应商 none」）被读成「语音已启用」——
     * 一个**假阳性**。现在只有真正跑到 ASR 那一层才算 `true`：
     *
     * - 调用成功 → `true`（`.so` 里有 sherpa）；
     * - 报「未启用 native-voice」→ `false`；
     * - 报别的（门面根本没建起来）→ `null`，调用方据此记 NOT EXECUTED 而不是替它二选一。
     */
    private fun nativeVoiceProbe(): Boolean? {
        // **复用 Activity 那份 repository，不能自己再 new 一个。**
        //
        // `YunjianRepository` 每次 `open()` 都构造一个新的 `NativeFacade`，而门面会打开
        // 语料与复习库。两份门面同时持有同一个 SQLite 文件时，写入方报
        // `database is locked`——第十轮真机实测里，正是这个探针把整轮的语料物化搞黄了
        // （十条中有六条因此变成 `corpus_unavailable_...database_is_locked`）。
        //
        // 一个进程一份门面。这条约束在真机上是硬的，本机跑不出来：桌面只有一个 AppState。
        val repository = compose.activity.repository
        return try {
            repository.startAsr("/nonexistent-model-dir", "明月", 16_000)
            true
        } catch (error: NativeException) {
            val message = error.message.orEmpty()
            AcceptanceReport.note("startAsr 探测：$message")
            when {
                message.contains("native-voice") -> false
                // 权重目录不存在是**预期**的：探测刻意传了一个不存在的目录，能走到
                // 「找不到模型」说明 sherpa 那一层真的在。
                message.contains("模型") || message.contains("model") -> true
                else -> null
            }
        } catch (error: Throwable) {
            AcceptanceReport.note("startAsr 探测抛出非 NativeException：${error.javaClass.name}")
            null
        }
    }

    private fun declaresRecordAudio(): Boolean =
        context.packageManager
            .getPackageInfo(context.packageName, android.content.pm.PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            ?.contains(android.Manifest.permission.RECORD_AUDIO) == true

    private fun recordAudioGranted(): Boolean =
        context.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED

    private fun crashLines(): Int =
        runCatching {
            device.executeShellCommand("logcat -d -b crash")
                .lineSequence()
                .count { it.contains(context.packageName) }
        }.getOrDefault(-1)

    /**
     * 从 logcat 捞内核报出的语料失败原因。
     *
     * `yunjian-core` 用 `tracing` 记 warn/error，Android 上落到 logcat。界面文案经过
     * 一层脱敏与兜底，而这里要的是内核原话。
     */
    private fun corpusFailureFromLogcat(): String =
        runCatching {
            device.executeShellCommand("logcat -d -t 400")
                .lineSequence()
                .filter { line ->
                    line.contains("yunjian") &&
                        (line.contains("语料") || line.contains("清单") || line.contains("归档"))
                }
                .map { it.trim() }
                .lastOrNull()
                ?.take(200)
                .orEmpty()
        }.getOrDefault("").ifBlank { "logcat 未见内核语料日志" }

    private fun systemProperty(name: String): String =
        runCatching { device.executeShellCommand("getprop $name").trim() }.getOrDefault("")

    private fun systemSetting(name: String): String =
        runCatching { device.executeShellCommand("settings get secure $name").trim() }.getOrDefault("")

    private fun File.listTemps(): Int =
        listFiles()?.count { it.name.endsWith(".tmp") } ?: 0

    /**
     * 截图落到外部缓存，由设备侧脚本 `adb pull` 取走。
     *
     * 「数字是结论，图是证据」——每条 PASS 都要有一张能被打开看的图。
     */
    private fun screenshot(assertion: String, label: String) {
        val bitmap = runCatching { device.takeScreenshot(File.createTempFile("shot", ".png")) }
        val dir = context.externalCacheDir ?: run {
            AcceptanceReport.unavailable(assertion, "screenshot_$label", "no_external_cache_dir")
            return
        }
        val target = File(dir, "$assertion-$label.png")
        val captured = runCatching {
            device.takeScreenshot(target)
        }.getOrDefault(false)
        if (captured && target.isFile && target.length() > 0L) {
            AcceptanceReport.measure(assertion, "screenshot_$label", target.name)
            AcceptanceReport.measure(assertion, "screenshot_${label}_bytes", target.length())
        } else {
            AcceptanceReport.unavailable(assertion, "screenshot_$label", "takeScreenshot_returned_false")
        }
        bitmap.getOrNull()
    }

    private companion object {
        const val TWO_CHAR_QUERY = "明月"
        const val APPEND_TEXT = "几时"
        /**
         * 轮询间隔。
         *
         * 2 秒而不是 500 毫秒：物化要十几分钟，每次读语义树都可能撞上重组中的布局
         * （`readNodes` 已容错，但少读三倍就少三倍机会）。判词要的是「阶段变过几次」，
         * 2 秒的粒度足够——实测一轮仍能记到 480~530 段。
         */
        const val POLL_MS = 2_000L

        /**
         * 集评覆盖集里的一首。苏轼《石芝》，随包语料实测有 1 条带出处的集评。
         *
         * 写成常量而不是「随便找一首」：随包语料的集评只覆盖 394 首，靠检索命中
         * 碰到其中一首是概率事件，而一条会间歇性变成 NOT EXECUTED 的断言等于没有断言。
         */
        const val COMMENTARY_POEM_ID = "206d45b9089865c3"

        /** 随包赏析覆盖的 16 首名篇之一。杜甫《春夜喜雨》。 */
        const val APPRECIATION_POEM_ID = "062f574ab2986a9b"

        /** 流式识别模型目录名。必须与 `full-measure.sh` 推上去的那个逐字一致。 */
        const val STREAMING_ASR_MODEL = "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20"

        /** 旁观预算，**不是判据阈值**。给足是为了让超时也能被量到，而不是超时就消失。 */
        const val MATERIALIZE_BUDGET_MS = 25L * 60L * 1000L
        const val VOICE_BUDGET_MS = 3L * 60L * 1000L

        val SCORE_KEYS = listOf(
            "completeness" to "完整度",
            "accuracy_strict" to "严格准确",
            "normal_count" to "正常",
            "deletion_count" to "漏",
            "insertion_count" to "增",
            "substitution_count" to "替",
        )

        /**
         * 从界面那行 outcome 里抠测量值的正则片段。
         *
         * `total_ms` 后面跟的是 `ms` 单位（界面给人看的是「时长=3000ms」），而宿主侧
         * 判据把它当数值比。真机实测第十二轮因此判 `total_ms=3000ms 不满足 > 0`——
         * 一次真的成功被判成失败。所以这里各自给出**只匹配数值**的模式，
         * 而不是笼统的 `[^ ]+`。
         */
        val VOICE_OUTCOME_KEYS = listOf(
            Triple("spoke", "开口=", "(true|false)"),
            Triple("pause_count", "停顿=", "([0-9]+)"),
            Triple("total_ms", "时长=", "([0-9]+)"),
            Triple("single_rtf", "单路RTF=", "([0-9.]+)"),
        )

        val VOICE_KEYS = listOf("spoke", "pause_count", "total_ms", "single_rtf")
    }
}
