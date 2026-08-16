package top.onethinker.yunjian

import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File

/**
 * 把实测值搬回宿主的唯一出口。
 *
 * # 三条通道，因为没有一条在所有配置下都可靠
 *
 * 1. **应用私有文件**：`run-as` 能读，但设备被 root 策略限制时读不到；
 * 2. **外部缓存**：`adb pull` 直接可取，但某些设备上 `getExternalCacheDir()` 为 `null`；
 * 3. **logcat**：一定能读，但长行会被截断。
 *
 * 三条都写，由设备侧脚本按可用性挑一条。这与 spike 的 `SpikeReport` 同一手法——那次真机
 * 上正是靠 logcat 兜住了前两条。
 *
 * # 为什么这里不判阈值
 *
 * 这个类**不认识** PASS 与 FAIL。它只报「量到了什么」。判定在宿主侧的
 * `xtask acceptance --platform android --set full` 完成，因为判词、阈值与报告 schema
 * 都在那里。让设备侧下结论等于把门禁搬到被测物内部。
 */
object AcceptanceReport {
    private const val TAG = "YunjianAcceptance"
    private const val PREFIX = "YUNJIAN-FULL"
    private const val FILE_NAME = "yunjian-acceptance.log"

    private val lines = mutableListOf<String>()

    fun measure(assertion: String, key: String, value: Any?) {
        emit("$PREFIX $assertion $key=${render(value)}")
    }

    /**
     * 这一项在本次运行中**没有测到**，并给出原因。
     *
     * 与 `measure(..., "")` 的区别是关键的：空串会被宿主侧读成「测到了一个空值」，
     * 进而记 FAIL——把一次未到达说成产品失败。spike 的 `SpikeCorpusTest` 已记过这条。
     */
    fun unavailable(assertion: String, key: String, reason: String) {
        emit("$PREFIX $assertion ${key}_unavailable=${reason.replace(' ', '_')}")
    }

    fun note(message: String) {
        emit("$PREFIX note ${message.replace('\n', ' ')}")
    }

    private fun render(value: Any?): String =
        when (value) {
            null -> "null"
            is Boolean, is Int, is Long, is Float, is Double -> value.toString()
            else -> value.toString().replace('\n', '/')
        }

    private fun emit(line: String) {
        Log.i(TAG, line)
        synchronized(lines) {
            lines += line
            flush()
        }
    }

    /**
     * 每行**追加**落盘。
     *
     * # 为什么是追加而不是重写整份
     *
     * 原先每次都把内存里的 `lines` 整份写下去。那在单进程里等价，但应用进程被杀（撤权、
     * 系统回收、被测代码崩溃）后 runner 会在新进程里继续跑，而新进程的 `lines` 是空的
     * ——于是第一次 `emit` 就把前面所有已经量到的值截掉。第一轮真机实测正是如此：
     * 前六条断言的测量值全部消失，报告里只剩最后三条，读起来像「那几条没跑」。
     *
     * 追加写让「已经量到的值」不依赖进程存活。重复行由宿主侧解析时按键覆盖，无害。
     */
    private fun flush() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val payload = lines.last() + "\n"
        runCatching { File(context.filesDir, FILE_NAME).appendText(payload) }
        runCatching {
            context.externalCacheDir?.let { dir -> File(dir, FILE_NAME).appendText(payload) }
        }
    }
}
