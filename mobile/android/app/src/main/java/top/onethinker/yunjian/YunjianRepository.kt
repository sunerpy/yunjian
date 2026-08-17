package top.onethinker.yunjian

import android.content.Context
import java.io.File
import org.json.JSONArray
import org.json.JSONObject
import top.yunjian.mobile.NativeEventSink
import top.yunjian.mobile.NativeFacade
import top.yunjian.mobile.fetchVoiceModel
import top.yunjian.mobile.materializeAssets

/**
 * 应用私有存储中的固定布局。
 *
 * 全部落在 `filesDir` 之下而不是外部存储：语料解压后有数 GiB，外部存储在
 * scoped storage 下需要额外权限，而且备份策略不同（manifest 已关 `allowBackup`）。
 */
class YunjianPaths(context: Context) {
    val appDataDir: File = File(context.filesDir, "yunjian")
    val corpusDataDir: File = File(appDataDir, "corpus")
    val schedulerPath: File = File(appDataDir, "recite.db")

    fun ensure() {
        corpusDataDir.mkdirs()
    }

    /** 语料是否已落在生产位置。首启判断只看这一件事。 */
    val corpusPresent: Boolean
        get() = File(corpusDataDir, "corpus.db").isFile
}

/** 首启物化的一条进度。字段与内核事件同名，不在这里另起口径。 */
data class MaterializationStage(
    val stage: String,
    val detail: String,
    val fraction: Float?,
)

/** 一条搜索结果。 */
data class SearchHit(
    val poemId: String,
    val title: String,
    val author: String,
    val snippet: String,
)

/** 集评一条，带出处。 */
data class Commentary(
    val text: String,
    val sourceTitle: String,
    val sourceLocator: String,
)

/** 随包 AI 赏析。`reviewed` 由 Rust 侧钉在 `false`，此处只读。 */
data class ShippedAppreciation(
    val text: String,
    val model: String,
    val source: String,
    val reviewed: Boolean,
)

/** 阅读页要显示的一切。 */
data class PoemReading(
    val poemId: String,
    val title: String,
    val author: String,
    val dynasty: String,
    val body: String,
    val commentaries: List<Commentary>,
    val appreciation: ShippedAppreciation?,
)

/** 一次打字背诵的题目。 */
data class ReciteSession(
    val poemId: String,
    val prompt: String,
    val lineCount: Int,
)

/** 一次打字背诵的评分。 */
data class ReciteScore(
    val completeness: Float,
    val accuracyStrict: Float,
    val accuracyLenient: Float,
    val isRejected: Boolean,
    val normalCount: Int,
    val deletionCount: Int,
    val insertionCount: Int,
    val substitutionCount: Int,
)

/**
 * 唯一允许触达 Rust 的地方。
 *
 * # 为什么把 JSON 解析集中在这里
 *
 * UniFFI 边界收发 `String` JSON（领域 serde 契约原样穿过）。把 `JSONObject` 散在各个
 * Composable 里，等于让每个界面各自猜一遍字段名；集中之后字段名只有一处会与 Rust 漂移，
 * 且漂移的表现是这里抛异常而不是某个界面静默显示空白。
 */
class YunjianRepository(private val context: Context) {
    private val paths = YunjianPaths(context)
    private var facade: NativeFacade? = null
    private val materialization = Materialization()

    val corpusPresent: Boolean
        get() = paths.corpusPresent

    private fun configJson(): String =
        JSONObject()
            .put("corpus_data_dir", paths.corpusDataDir.absolutePath)
            .put("scheduler_path", paths.schedulerPath.absolutePath)
            .put("app_data_dir", paths.appDataDir.absolutePath)
            // `none` 是「不配置任何生成供应商」。随包赏析走 `shippedAppreciation`，
            // 那条路径不需要 key，所以首启就能显示赏析。
            .put("provider", "none")
            .toString()

    /**
     * 走生产路径下载、校验并原子物化语料与随包赏析种子。
     *
     * 进度以回调逐条送出；调用方**必须**持续拉取直到 `Done` 或 `Failed`。
     * 把一次轮询超时当成终态会让首启在解压中途被误判为完成。
     *
     * **进程内至多一次真的在跑**（见 [`Materialization`]）。重复调用只会登记为订阅者并
     * 收到已知状态的回放——两次并发物化会在 corpus.db 上撞成 `database is locked`。
     */
    fun materialize(
        onStage: (MaterializationStage) -> Unit,
        onDone: (String?) -> Unit,
    ) {
        if (!materialization.claim(onStage, onDone)) {
            return
        }
        paths.ensure()
        val operation = materializeAssets(configJson())
        operation.subscribe(
            object : NativeEventSink {
                override fun onEvent(eventJson: String) {
                    val event = JSONObject(eventJson)
                    when (event.optString("type")) {
                        "progress" -> materialization.publishStage(readStage(event.getJSONObject("payload")))
                        "item" -> materialization.publishStage(readSummary(event.getJSONObject("payload")))
                        "done" -> materialization.publishDone(null)
                        // `Event` 是**邻接标签**（`#[serde(tag="type", content="payload")]`），
                        // 所以 `Failed { message }` 的 message 在 `payload` 里，不在顶层。
                        // 读顶层拿到空串，于是真因被兜底文案「语料物化失败」顶掉——真机上
                        // 界面只显示那句话，排查无从下手。桌面在 PR #108 记过同一层标签陷阱。
                        "failed" ->
                            materialization.publishDone(
                                event.optJSONObject("payload")?.optString("message")
                                    ?.takeIf { it.isNotBlank() }
                                    ?: "语料物化失败（后端未给出原因）",
                            )
                        "cancelled" -> materialization.publishDone("语料物化已取消")
                    }
                }
            },
        )
    }

    private fun readStage(payload: JSONObject): MaterializationStage {
        val stage = payload.optString("stage")
        return when (stage) {
            "already_present" -> MaterializationStage(stage, "语料已在本地", 1f)
            "verifying_archive" ->
                MaterializationStage(
                    stage,
                    "正在核对归档摘要 · ${mib(payload.optLong("bytes"))}",
                    null,
                )
            "archive_verified" -> MaterializationStage(stage, "归档摘要一致", null)
            "decompressing" -> {
                val done = payload.optLong("bytes_done")
                val total = payload.optLong("bytes_total")
                MaterializationStage(
                    stage,
                    // `total == 0` 表示清单没给解压后大小，真机上就是这种情况
                    // （实测「正在解压语料库 · 128.4 MiB / 0.0 MiB」）。分母写 0
                    // 比不写更糟：它看起来像一个已知的总量，而那个总量是假的。
                    if (total > 0L) {
                        "正在解压语料库 · ${mib(done)} / ${mib(total)}"
                    } else {
                        "正在解压语料库 · 已写出 ${mib(done)}"
                    },
                    if (total > 0L) done.toFloat() / total.toFloat() else null,
                )
            }
            "materialized" ->
                MaterializationStage(stage, "语料已原子落地 · ${payload.optString("corpus_version")}", null)
            "deriving" -> {
                val done = payload.optLong("done")
                val total = payload.optLong("total")
                val step = payload.optString("step")
                MaterializationStage(
                    stage,
                    if (total > 0L) "$step（$done / $total 首）" else step,
                    if (total > 0L) done.toFloat() / total.toFloat() else null,
                )
            }
            "derive_failed" ->
                MaterializationStage(
                    stage,
                    "首启派生未完成：两字查询会退化，下次启动重试",
                    null,
                )
            "ready" ->
                MaterializationStage(
                    stage,
                    "语料就绪 · ${payload.optString("corpus_version")}" +
                        if (payload.optBoolean("derived")) " · 派生索引就绪" else " · 派生索引缺失",
                    1f,
                )
            else -> MaterializationStage(stage, stage, null)
        }
    }

    private fun readSummary(payload: JSONObject): MaterializationStage =
        MaterializationStage(
            "summary",
            "${payload.optLong("poem_count")} 首 · 随包赏析 ${payload.optInt("shipped_records")} 条",
            1f,
        )

    private fun mib(bytes: Long): String = "%.1f MiB".format(bytes.toDouble() / (1024.0 * 1024.0))

    /** 惰性构造门面。语料不在时构造必然失败，所以由调用方先完成物化。 */
    fun open(): NativeFacade {
        facade?.let { return it }
        val created = NativeFacade(configJson())
        facade = created
        return created
    }

    fun corpusStatusJson(): String = open().corpusStatus()

    /**
     * 正文检索。字段名取自 `TextSearchRequest` / `SearchPage` / `TextSearchHit`
     * 的 serde 契约：命中数组是 `hits`（不是 `items`），命中句在 `snippet.text`。
     */
    fun searchText(query: String, limit: Int = 20): List<SearchHit> {
        val request = JSONObject().put("query", query).put("limit", limit)
        val page = JSONObject(open().searchText(request.toString()))
        return page.optJSONArray("hits").objects().map { hit ->
            SearchHit(
                poemId = hit.optString("poem_id"),
                title = hit.optString("title"),
                author = hit.optString("author"),
                snippet = hit.optJSONObject("snippet")?.optString("text").orEmpty(),
            )
        }
    }

    /**
     * 阅读页数据。
     *
     * 集评的出处在 `citation` 对象里（`work` / `author` / `dynasty.canonical` /
     * `work_completed_by` / `source_note`），而且 `citation` **不是** `Option`：
     * 缺出处的集评根本不会构造出来，内核直接返回 `CommentaryCitationMissing`。
     * 因此这里不需要「没有出处怎么显示」这条分支。
     */
    fun reading(poemId: String): PoemReading {
        val detail = JSONObject(open().poemDetail(JSONObject().put("poem_id", poemId).toString()))
        val poem = detail.getJSONObject("poem")
        val commentaries = detail.optJSONArray("commentaries").objects().map { entry ->
            val citation = entry.getJSONObject("citation")
            val dynasty = citation.optJSONObject("dynasty")?.optString("canonical").orEmpty()
            Commentary(
                text = entry.optString("text"),
                sourceTitle = "$dynasty·${citation.optString("author")}《${citation.optString("work")}》",
                sourceLocator =
                    "${citation.optString("source_note")}（成书不晚于 ${citation.optInt("work_completed_by")}）",
            )
        }
        return PoemReading(
            poemId = poemId,
            title = poem.optString("title"),
            author = poem.optString("author"),
            dynasty = poem.optJSONObject("dynasty")?.optString("canonical").orEmpty(),
            body = poem.optString("body"),
            commentaries = commentaries,
            appreciation = shippedAppreciation(poemId),
        )
    }

    /** 随包赏析。命中不需要 API key；未命中返回 `null` 而不是抛错。 */
    fun shippedAppreciation(poemId: String): ShippedAppreciation? {
        val raw = open().shippedAppreciation(poemId, "shipped") ?: return null
        val value = JSONObject(raw)
        return ShippedAppreciation(
            text = value.optString("text"),
            model = value.optString("model"),
            source = value.optString("source"),
            reviewed = value.optBoolean("reviewed"),
        )
    }

    fun reciteStart(poemId: String): ReciteSession {
        val request = JSONObject()
            .put("poem_id", poemId)
            .put("mode", JSONObject().put("mode", "first_char"))
        val session = JSONObject(open().reciteStart(request.toString()))
        return ReciteSession(
            poemId = poemId,
            prompt = session.optString("prompt"),
            lineCount = session.optInt("line_count"),
        )
    }

    fun reciteSubmit(poemId: String, answer: String, grade: String): ReciteScore {
        val request = JSONObject()
            .put("poem_id", poemId)
            .put("answer", answer)
            .put("grade", grade)
        val score = JSONObject(open().reciteSubmit(request.toString())).getJSONObject("score")
        return ReciteScore(
            completeness = score.optDouble("completeness").toFloat(),
            accuracyStrict = score.optDouble("accuracy_strict").toFloat(),
            accuracyLenient = score.optDouble("accuracy_lenient").toFloat(),
            isRejected = score.optBoolean("is_rejected"),
            normalCount = score.optInt("normal_count"),
            deletionCount = score.optInt("deletion_count"),
            insertionCount = score.optInt("insertion_count"),
            substitutionCount = score.optInt("substitution_count"),
        )
    }

    /**
     * 按需下载并校验一个语音模型，返回它在设备上的目录。
     *
     * 走产品自己那条路径（`ModelCache::ensure`）：下载 + SHA-256 校验 + 原子解包。
     * 由 Rust 写文件，属主就是应用自己——外部工具塞进来的文件属主是 `shell`，
     * 应用读不到（真机实测）。
     */
    fun fetchVoiceModel(modelName: String, onStage: (String) -> Unit): String? {
        val root = File(modelRoot(), "models").apply { mkdirs() }
        val operation = fetchVoiceModel(root.absolutePath, modelName)
        var directory: String? = null
        var failure: String? = null
        while (true) {
            val raw = operation.nextEvent(500uL) ?: continue
            val event = JSONObject(raw)
            when (event.optString("type")) {
                "progress" -> onStage(describeModelStage(event.optJSONObject("payload")))
                "item" -> directory = event.optJSONObject("payload")?.optString("directory")
                "done" -> return directory
                "failed" -> {
                    failure = event.optJSONObject("payload")?.optString("message")
                    onStage("模型下载失败：${failure.orEmpty()}")
                    return null
                }
                "cancelled" -> return null
            }
        }
    }

    private fun describeModelStage(payload: JSONObject?): String =
        when (payload?.optString("stage")) {
            "downloading" -> {
                val done = payload.optLong("bytes_done")
                val total = payload.optLong("bytes_total")
                if (total > 0L) {
                    "正在下载语音模型 · ${mib(done)} / ${mib(total)}"
                } else {
                    "正在下载语音模型 · 已写出 ${mib(done)}"
                }
            }
            "verifying" -> "正在核对模型摘要 · ${mib(payload.optLong("bytes"))}"
            "verified" -> "模型摘要一致"
            "unpacking" -> "正在解包语音模型"
            else -> "准备语音模型"
        }

    /** 模型落点的根。与 `MainActivity` 同一口径：外部私有目录优先。 */
    fun modelRoot(): File = File(context.getExternalFilesDir(null) ?: context.filesDir, "yunjian")

    /**
     * 启动真实 sherpa 双路 ASR。
     *
     * 返回句柄由调用方持有：PCM 由 Kotlin 的 `AudioRecord` 逐帧送入，这正是
     * UniFFI 分支被裁决选中的理由（Tauri 外壳拿不到原生采集）。
     */
    /**
     * 启动流式识别。
     *
     * `int8 = true` 是移动端的取值：fp32 那套权重 531 MiB，int8 是 189 MiB，而手机上
     * 内存与存储都比桌面紧。`TransducerFiles::discover` 按这个开关挑 `.int8.onnx` 还是
     * `.onnx`，**挑错会报「找不到 encoder」而不是「精度不对」**，那条报错不指向真因。
     */
    fun startAsr(modelDir: String, reference: String, sampleRate: Int) =
        open().startAsr(modelDir, true, reference, sampleRate.toUInt())

    fun close() {
        facade = null
    }

    /**
     * 首启物化在**进程内**的唯一状态。
     *
     * # 为什么守卫不能挂在 ViewModel 上
     *
     * `MainViewModel` 属于 Activity：每次 `onCreate` 都拿到一个状态回到 `Idle` 的新实例
     * （instrumentation 逐条测试各建一次界面、旋屏、进程回收后返回都会这样）。于是
     * 「已经在跑」这件事在下一个 ViewModel 眼里不存在，第二次物化启动，与上一次仍在写
     * corpus.db 的 Rust 线程撞成 **`database is locked`**。
     *
     * 真机实测这是一个**竞态**：第十六轮 t01→t02 侥幸没撞上（t02 拿到 489 段进度），
     * 第十七、十八轮撞上了，十条里六条被连带拖成 NOT EXECUTED，而报错文字完全不提
     * 「有两次物化」。所以状态必须活在进程里，与 Activity 生命周期解耦。
     *
     * **同时要回放。** 只是「第二次直接返回」会让重建后的界面停在「尚未下载语料库」，
     * 而后台其实正在解压——那句陈旧的话比空白更糟。新订阅者进来时先收到已知的最新阶段，
     * 已终结时立刻收到终态。
     */
    private class Materialization {
        private val lock = Any()
        private var started = false
        private var finished = false
        private var failure: String? = null
        private var lastStage: MaterializationStage? = null
        private val listeners = mutableListOf<Listener>()

        private data class Listener(
            val onStage: (MaterializationStage) -> Unit,
            val onDone: (String?) -> Unit,
        )

        /** 登记订阅者并回放已知状态；返回 `true` 表示**本次调用**要真的去跑。 */
        fun claim(onStage: (MaterializationStage) -> Unit, onDone: (String?) -> Unit): Boolean {
            val replayStage: MaterializationStage?
            val replayTerminal: Boolean
            val replayFailure: String?
            val shouldRun: Boolean
            synchronized(lock) {
                if (!finished) {
                    listeners += Listener(onStage, onDone)
                }
                replayStage = lastStage
                replayTerminal = finished
                replayFailure = failure
                shouldRun = !started
                started = true
            }
            // 回调在锁外调用：订阅者可能同步回读状态，持锁调用会自锁。
            replayStage?.let(onStage)
            if (replayTerminal) {
                onDone(replayFailure)
            }
            return shouldRun
        }

        fun publishStage(stage: MaterializationStage) {
            val snapshot = synchronized(lock) {
                lastStage = stage
                listeners.toList()
            }
            snapshot.forEach { it.onStage(stage) }
        }

        fun publishDone(reason: String?) {
            val snapshot = synchronized(lock) {
                finished = true
                failure = reason
                val current = listeners.toList()
                listeners.clear()
                current
            }
            snapshot.forEach { it.onDone(reason) }
        }

        // **失败在本进程内是终态，刻意不自动重试。**
        // 自动重试会让「每次 Activity 重建都再试一次」，而上一次可能仍在跑——那正是本类
        // 要消除的并发。一次网络抖动导致后续断言报「语料不可用」是**如实上报**，
        // 不是需要被掩盖的东西（第十四轮 `Peer disconnected` 就是这样，判读正确）。
        // 要重试就重启进程，那也是用户真实的处置方式。
    }

    companion object {
        @Volatile
        private var instance: YunjianRepository? = null

        /**
         * 进程内唯一那份 repository。
         *
         * # 为什么必须是单例
         *
         * 每份 repository 会惰性构造自己的 `NativeFacade`，而门面打开语料与复习库两个
         * SQLite 文件。两份门面同时持有同一个文件时，写入方报 `database is locked`——
         * 真机实测里这条错误让十条断言中的六条被连带拖红，而它的文字完全不提「有两份」。
         *
         * 用 `applicationContext` 而不是 Activity context：单例活到进程结束，持 Activity
         * 会泄漏一个已销毁的对象。
         */
        fun shared(context: Context): YunjianRepository {
            instance?.let { return it }
            return synchronized(this) {
                instance ?: YunjianRepository(context.applicationContext).also { instance = it }
            }
        }
    }
}

private fun JSONArray?.objects(): List<JSONObject> {
    if (this == null) return emptyList()
    return (0 until length()).mapNotNull { optJSONObject(it) }
}
