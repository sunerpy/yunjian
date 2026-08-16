package top.onethinker.yunjian

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.core.content.ContextCompat
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** 首启物化的三态。 */
sealed interface CorpusState {
    data object Idle : CorpusState
    data class Working(val stage: MaterializationStage) : CorpusState
    data class Ready(val facts: String) : CorpusState
    data class Failed(val reason: String) : CorpusState
}

/** 语音一轮的可观测状态。 */
sealed interface VoiceState {
    data object Idle : VoiceState
    data class Listening(val detail: String) : VoiceState
    data class Finished(val detail: String) : VoiceState

    /**
     * 语音不可用，已降级到打字。
     *
     * `reason` 必须是**具体原因**而不是「语音不可用」：真机上这条断言检的正是
     * 「显示具体原因」，一句笼统的话与不显示等价。
     */
    data class Degraded(val reason: String) : VoiceState
}

data class UiState(
    val corpus: CorpusState = CorpusState.Idle,
    val query: String = "",
    val directId: String = "",
    val hits: List<SearchHit> = emptyList(),
    val searched: Boolean = false,
    val reading: PoemReading? = null,
    val reciteSession: ReciteSession? = null,
    val reciteAnswer: String = "",
    val reciteScore: ReciteScore? = null,
    val voice: VoiceState = VoiceState.Idle,
    val error: String? = null,
)

class MainViewModel(private val repository: YunjianRepository) : ViewModel() {
    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /**
     * 首启物化。
     *
     * 语料已在本地时**也走这条路径**：`AssetResolver` 的 `already_present` 分支会立刻
     * 报出来，界面因此总有话说。跳过它会让「第二次启动界面一片空白」成为一个正常状态。
     */
    fun materialize() {
        // 三态都要挡：`Working` 是正在跑，`Ready` 是已经跑完。只挡 `Working` 时
        // Activity 重建会在已就绪之后再跑一次，而那一次与仍持有句柄的门面撞成
        // `database is locked`（真机实测）。
        if (_state.value.corpus is CorpusState.Working || _state.value.corpus is CorpusState.Ready) {
            return
        }
        _state.update { it.copy(corpus = CorpusState.Working(MaterializationStage("starting", "正在联系发布地址", null))) }
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                repository.materialize(
                    onStage = { stage ->
                        _state.update { current ->
                            if (stage.stage == "summary") {
                                current.copy(corpus = CorpusState.Ready(stage.detail))
                            } else {
                                current.copy(corpus = CorpusState.Working(stage))
                            }
                        }
                    },
                    onDone = { failure ->
                        _state.update { current ->
                            when {
                                failure != null -> current.copy(corpus = CorpusState.Failed(failure))
                                current.corpus is CorpusState.Ready -> current
                                else -> current.copy(corpus = CorpusState.Ready(readFacts()))
                            }
                        }
                    },
                )
            }.onFailure { error ->
                // 带上异常类名：`message` 可能为 null（`UnsatisfiedLinkError` 常见），
                // 那时只写一句「语料物化失败」等于把真因丢掉。
                _state.update {
                    it.copy(
                        corpus = CorpusState.Failed(
                            error.message ?: "${error.javaClass.name}（无 message）",
                        ),
                    )
                }
            }
        }
    }

    private fun readFacts(): String =
        runCatching { repository.corpusStatusJson() }.getOrElse { "语料状态不可读：${it.message}" }

    fun onQueryChange(value: String) {
        _state.update { it.copy(query = value) }
    }

    fun onDirectIdChange(value: String) {
        _state.update { it.copy(directId = value) }
    }

    fun search() {
        val query = _state.value.query.trim()
        if (query.isEmpty()) return
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { repository.searchText(query) } }
            result
                .onSuccess { hits -> _state.update { it.copy(hits = hits, searched = true, error = null) } }
                .onFailure { error -> _state.update { it.copy(error = error.message, searched = true) } }
        }
    }

    fun openReading(poemId: String) {
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { repository.reading(poemId) } }
            result
                .onSuccess { reading -> _state.update { it.copy(reading = reading, error = null) } }
                .onFailure { error -> _state.update { it.copy(error = error.message) } }
        }
    }

    fun startRecite(poemId: String) {
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { repository.reciteStart(poemId) } }
            result
                .onSuccess { session ->
                    _state.update {
                        // 预填正文：「向已有内容的字段输入」这条断言要求字段一开始就不为空。
                        // 空字段测不出「输入法在已有文本上追加」这件事。
                        it.copy(reciteSession = session, reciteAnswer = "明月", reciteScore = null, error = null)
                    }
                }
                .onFailure { error -> _state.update { it.copy(error = error.message) } }
        }
    }

    fun onReciteAnswerChange(value: String) {
        _state.update { it.copy(reciteAnswer = value) }
    }

    fun submitRecite(grade: String = "good") {
        val session = _state.value.reciteSession ?: return
        val answer = _state.value.reciteAnswer
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching { repository.reciteSubmit(session.poemId, answer, grade) }
            }
            result
                .onSuccess { score -> _state.update { it.copy(reciteScore = score, error = null) } }
                .onFailure { error -> _state.update { it.copy(error = error.message) } }
        }
    }

    /**
     * 一轮语音跟读。
     *
     * 三条降级路径都写成 [VoiceState.Degraded] 并带具体原因：
     *
     * 1. 未授予 `RECORD_AUDIO`；
     * 2. 本次原生库未启用 `native-voice`；
     * 3. ASR 权重目录不在（模型按需下载，未下载时不是缺陷）。
     *
     * **刻意不做的事**：不把识别结果送进评分。2026-08-11 裁决按 1800 句实测 CER 77.01%
     * 定下 `guided_practice`——只报「是否开口／停顿／相对节奏」，FSRS 等级由用户自选。
     */
    fun startVoiceRound(context: Context, poemId: String, reference: String, modelDir: String) {
        val granted = ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        if (!granted) {
            _state.update {
                it.copy(
                    voice = VoiceState.Degraded("未授予麦克风权限（android.permission.RECORD_AUDIO 被拒绝）；已切到打字背诵"),
                )
            }
            startRecite(poemId)
            return
        }
        if (!java.io.File(modelDir).isDirectory) {
            _state.update {
                it.copy(voice = VoiceState.Degraded("ASR 权重目录不存在：$modelDir；已切到打字背诵"))
            }
            startRecite(poemId)
            return
        }
        // `checkSelfPermission` 说已授予**不等于**真能采到音。运行时权限之外还有一层
        // appops：`android:record_audio` 被拒时 `AudioRecord` 照样能建、`startRecording`
        // 也不报错，读到的是**静音流**。只看权限的产品会在这种设备状态下静静录一段空白，
        // 用户看到「正在采集」却永远没有结果——比明确降级更糟。
        // 所以先探一小段，全零即按「采集被拒」降级并说明。
        val silent = probeSilentCapture()
        if (silent != null) {
            _state.update { it.copy(voice = VoiceState.Degraded(silent)) }
            startRecite(poemId)
            return
        }
        _state.update { it.copy(voice = VoiceState.Listening("正在采集")) }
        viewModelScope.launch(Dispatchers.IO) {
            runCatching { runVoiceRound(reference, modelDir) }
                .onSuccess { detail -> _state.update { it.copy(voice = VoiceState.Finished(detail)) } }
                .onFailure { error ->
                    _state.update {
                        it.copy(voice = VoiceState.Degraded("${error.message ?: "语音会话失败"}；已切到打字背诵"))
                    }
                    startRecite(poemId)
                }
        }
    }

    /**
     * 采集能否真的拿到非静音数据。可用时返回 `null`，否则返回可直接展示的原因。
     *
     * 判据是「这一小段里有没有任何非零采样」，不是能量阈值：静音流是**逐字节全零**，
     * 而环境噪声哪怕极轻也不会全零。用阈值反而会把安静房间里的真采集误判成被拒。
     */
    private fun probeSilentCapture(): String? {
        val minBuffer = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL, ENCODING)
        if (minBuffer <= 0) {
            return "音频采集不可用：AudioRecord.getMinBufferSize 返回 $minBuffer；已切到打字背诵"
        }
        val recorder = AudioRecord(
            MediaRecorder.AudioSource.MIC,
            SAMPLE_RATE,
            CHANNEL,
            ENCODING,
            maxOf(minBuffer, FRAME_SAMPLES * 2 * 4),
        )
        try {
            if (recorder.state != AudioRecord.STATE_INITIALIZED) {
                return "音频采集不可用：AudioRecord 未初始化（state=${recorder.state}）；已切到打字背诵"
            }
            recorder.startRecording()
            if (recorder.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                return "音频采集被拒：startRecording 未进入录音状态；已切到打字背诵"
            }
            val buffer = ShortArray(FRAME_SAMPLES)
            var nonZero = 0
            var read = 0
            repeat(SILENCE_PROBE_FRAMES) {
                val n = recorder.read(buffer, 0, buffer.size)
                if (n > 0) {
                    read += n
                    nonZero += (0 until n).count { index -> buffer[index] != 0.toShort() }
                }
            }
            if (read == 0) {
                return "音频采集被拒：读不到任何采样（RECORD_AUDIO 的 appops 可能为 deny）；已切到打字背诵"
            }
            if (nonZero == 0) {
                return "音频采集被拒：$read 个采样全为静音，RECORD_AUDIO 权限或 appops 未真正放开；已切到打字背诵"
            }
            return null
        } finally {
            runCatching { recorder.stop() }
            recorder.release()
        }
    }

    private fun runVoiceRound(reference: String, modelDir: String): String {
        val operation = repository.startAsr(modelDir, reference, SAMPLE_RATE)
        val minBuffer = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL, ENCODING)
        val recorder = AudioRecord(
            MediaRecorder.AudioSource.MIC,
            SAMPLE_RATE,
            CHANNEL,
            ENCODING,
            maxOf(minBuffer, FRAME_SAMPLES * 2 * 4),
        )
        val outcome = StringBuilder()
        try {
            recorder.startRecording()
            val buffer = ShortArray(FRAME_SAMPLES)
            var pushed = 0
            while (pushed < FRAMES_PER_ROUND) {
                val read = recorder.read(buffer, 0, buffer.size)
                if (read <= 0) break
                operation.pushPcm(FloatArray(read) { index -> buffer[index] / 32768f }.toList())
                pushed += 1
            }
            operation.finishInput()
            // 持续拉取直到唯一终态。一次轮询超时（`null`）不是终态——把它当终态会让
            // 语音一轮在识别还没吐出 outcome 时就被判成结束。
            while (true) {
                val raw = operation.nextEvent(POLL_MS.toULong()) ?: continue
                val event = org.json.JSONObject(raw)
                when (event.optString("type")) {
                    "item" -> outcome.append(describeItem(event.getJSONObject("payload")))
                    "done" -> return outcome.ifEmptyDefault()
                    "failed" -> throw IllegalStateException(event.optString("message"))
                    "cancelled" -> throw IllegalStateException("语音会话已取消")
                }
            }
        } finally {
            runCatching { recorder.stop() }
            recorder.release()
            operation.shutdown()
        }
    }

    private fun describeItem(payload: org.json.JSONObject): String =
        when (payload.optString("type")) {
            "outcome" ->
                "开口=${payload.optBoolean("spoke")} 停顿=${payload.optInt("pause_count")} " +
                    "时长=${payload.optLong("total_ms")}ms 单路RTF=${payload.optDouble("single_rtf")}"
            else -> ""
        }

    private fun StringBuilder.ifEmptyDefault(): String =
        if (isBlank()) "识别结束但未产出 outcome" else toString()

    fun dismissError() {
        _state.update { it.copy(error = null) }
    }

    private companion object {
        const val SAMPLE_RATE = 16_000
        const val CHANNEL = AudioFormat.CHANNEL_IN_MONO
        const val ENCODING = AudioFormat.ENCODING_PCM_16BIT
        const val FRAME_SAMPLES = 1_600
        const val FRAMES_PER_ROUND = 30
        const val POLL_MS = 200L

        /** 静音探测读几帧。3 帧 × 1600 采样 ≈ 300 ms，足够区分全零流与真采集。 */
        const val SILENCE_PROBE_FRAMES = 3
    }
}
