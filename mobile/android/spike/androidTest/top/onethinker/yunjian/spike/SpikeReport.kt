package top.onethinker.yunjian.spike

import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File

/**
 * 判据测量的唯一出口。
 *
 * 三条通道刻意都写，因为它们的失效方式不同：
 * - **应用私有文件**（`filesDir`）：`adb shell run-as` 可读，容量不受限，是主通道。
 * - **外部缓存**（`externalCacheDir`）：`adb shell cat` 直读，不需要 `run-as`；
 *   Device Farm 宿主侧最省事的一条。
 * - **logcat**：不需要任何文件系统约定，但判据②会在同一轮里下载 223 MiB 并进入首启派生，
 *   那段输出足以把 logcat 环形缓冲冲掉。所以 logcat 只当兜底，不当依据。
 *
 * 行格式与 `xtask/src/acceptance/measurements.rs` 的解析器逐字对应：
 * `YUNJIAN-MEASURE <criterion> <key>=<value>`，测不到的写
 * `YUNJIAN-MEASURE-UNAVAILABLE <criterion> <key> reason=<why>`。两种前缀不同不是风格问题：
 * 若不可用项复用同一前缀而把值写空，「没测到」与「测到空串」在解析后不可区分，
 * 而前者不该影响 verdict、后者应当判 FAIL。
 */
object SpikeReport {
    private const val TAG = "YunjianSpike"
    private const val FILE_NAME = "yunjian-measure.log"

    private val sinks: List<File> by lazy {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        listOfNotNull(File(context.filesDir, FILE_NAME), context.externalCacheDir?.let { File(it, FILE_NAME) })
            .mapNotNull { candidate ->
                runCatching {
                    candidate.parentFile?.mkdirs()
                    candidate
                }.getOrNull()
            }
    }

    fun measure(criterion: String, key: String, value: Any?) {
        emit("YUNJIAN-MEASURE $criterion $key=${render(value)}")
    }

    fun unavailable(criterion: String, key: String, reason: String) {
        emit("YUNJIAN-MEASURE-UNAVAILABLE $criterion $key reason=$reason")
    }

    fun note(message: String) {
        emit("# $message")
    }

    /** 设备型号与 OS build 是每条判据都必须记录的，集中在这里避免三个测试类各写一遍。 */
    fun identify(criterion: String) {
        measure(criterion, "device_model", android.os.Build.MODEL)
        measure(criterion, "os_build", "${android.os.Build.VERSION.RELEASE}/${android.os.Build.VERSION.SDK_INT}")
    }

    private fun emit(line: String) {
        // 换行会把一行测量拆成两行，解析器读到的后半段就成了噪声。
        val safe = line.replace('\n', ' ').replace('\r', ' ')
        Log.i(TAG, safe)
        sinks.forEach { sink ->
            runCatching { sink.appendText("$safe\n") }
        }
    }

    private fun render(value: Any?): String =
        when (value) {
            null -> ""
            is Boolean -> if (value) "true" else "false"
            else -> value.toString()
        }
}
