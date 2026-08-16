package top.onethinker.yunjian.spike

import android.provider.Settings
import android.view.inputmethod.EditorInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.json.JSONObject
import org.junit.Test
import org.junit.runner.RunWith

/**
 * 判据③：`targetSdk 35` 的边到边窗口里，用中文输入法向贴底检索框输入中文，
 * 键盘不得遮挡输入框，且 `visualViewport` 必须随键盘更新。
 *
 * ## 文字是怎么进去的
 *
 * Device Farm 喂不进拼音候选选择，`adb shell input text` 也送不了非 ASCII。
 * 所以走 IME 自己的 API：`WebView.onCreateInputConnection(...).commitText(...)`，
 * 这正是任何输入法把候选词交给编辑框时调用的那条路。它拿不到连接时退到无障碍的
 * `ACTION_SET_TEXT`。**用了哪条路会记进 `ime_commit_path`**——不写明这一点，
 * 一个 PASS 就无法判断它证明了什么。
 *
 * 软键盘由真正的 Gboard 弹出（PR #99 实测该设备 147 个 subtype 里有且仅有 `zh_CN`），
 * 因此遮挡与 viewport 两项量的是真实 IME 窗口，不是模拟的插入值。
 */
@RunWith(AndroidJUnit4::class)
class SpikeImeTest {

    @Test
    fun types_chinese_into_a_search_field_without_the_keyboard_covering_it() {
        SpikeReport.identify(CRITERION)
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        SpikeReport.measure(
            CRITERION,
            "target_sdk",
            context.packageManager.getApplicationInfo(context.packageName, 0).targetSdkVersion,
        )
        SpikeReport.measure(
            CRITERION,
            "default_ime",
            Settings.Secure.getString(context.contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD),
        )
        SpikeReport.measure(CRITERION, "app_locales", context.resources.configuration.locales.toLanguageTags())

        ActivityScenario.launch(SpikeWebViewActivity::class.java).use { scenario ->
            val device = UiDevice.getInstance(instrumentation)
            device.waitForIdle()
            var activity: SpikeWebViewActivity? = null
            scenario.onActivity { activity = it }
            val target = activity
            if (target == null) {
                unavailableAll("spike_activity_never_resumed")
                return
            }
            // 页面加载完才有输入框可聚焦。轮询 probe 比固定 sleep 稳：真机上首帧到
            // JS 就绪之间的间隔跨度很大，固定等待要么不够要么白等。
            val before = awaitProbe(target, instrumentation) ?: run {
                unavailableAll("webview_probe_never_ready")
                return
            }
            SpikeReport.measure(CRITERION, "has_visual_viewport", before.optBoolean("hasVisualViewport"))
            SpikeReport.measure(CRITERION, "viewport_height_before", before.optInt("viewportHeight"))

            focusInput(target, instrumentation)
            val shown = awaitKeyboard(target, device)
            SpikeReport.measure(CRITERION, "keyboard_shown", shown)
            SpikeReport.measure(CRITERION, "ime_bottom_inset_px", target.imeBottomInset)
            if (!shown) {
                // 键盘没弹出时遮挡与 viewport 无从测量。这不是产品失败，是本轮没执行到。
                for (key in listOf("keyboard_overlap_px", "input_visible", "visual_viewport_updated")) {
                    SpikeReport.unavailable(CRITERION, key, "soft_keyboard_never_appeared")
                }
                SpikeReport.unavailable(CRITERION, "entered_text", "soft_keyboard_never_appeared")
                return
            }

            val path = commitChinese(target, instrumentation)
            SpikeReport.measure(CRITERION, "ime_commit_path", path)
            val after = awaitProbe(target, instrumentation) ?: run {
                unavailableAll("webview_probe_unreadable_after_input")
                return
            }
            record(target, before, after)
        }
    }

    private fun record(
        activity: SpikeWebViewActivity,
        before: JSONObject,
        after: JSONObject,
    ) {
        val geometry = activity.geometry()
        SpikeReport.measure(CRITERION, "edge_to_edge", geometry.edgeToEdge)
        SpikeReport.measure(CRITERION, "display_height_px", geometry.displayHeight)
        SpikeReport.measure(CRITERION, "system_bars_bottom_px", geometry.systemBarsBottomInset)
        SpikeReport.measure(CRITERION, "webview_top_px", geometry.webTop)
        SpikeReport.measure(CRITERION, "webview_height_px", geometry.webHeight)
        SpikeReport.measure(CRITERION, "device_pixel_ratio", after.optDouble("ratio", -1.0))

        val inputBottomOnScreen = geometry.webTop + after.optInt("bottomPx")
        val inputTopOnScreen = geometry.webTop + after.optInt("topPx")
        val overlap = maxOf(0, inputBottomOnScreen - geometry.keyboardTop)
        SpikeReport.measure(CRITERION, "input_bottom_screen_px", inputBottomOnScreen)
        SpikeReport.measure(CRITERION, "keyboard_top_screen_px", geometry.keyboardTop)
        SpikeReport.measure(CRITERION, "keyboard_overlap_px", overlap)
        SpikeReport.measure(
            CRITERION,
            "input_visible",
            overlap == 0 && inputTopOnScreen >= geometry.webTop && inputBottomOnScreen <= geometry.displayHeight,
        )

        val heightBefore = before.optInt("viewportHeight", -1)
        val heightAfter = after.optInt("viewportHeight", -1)
        SpikeReport.measure(CRITERION, "viewport_height_after", heightAfter)
        SpikeReport.measure(CRITERION, "viewport_samples", after.optJSONArray("viewportSamples")?.toString() ?: "[]")
        // 「更新过」必须由**两次取值不同**证明。只读一个最终值无法区分「随键盘缩小了」
        // 与「本来就是这个高度、事件从未派发」，而后者正是 visualViewport 那个已知缺陷。
        SpikeReport.measure(
            CRITERION,
            "visual_viewport_updated",
            heightBefore > 0 && heightAfter > 0 && heightAfter < heightBefore,
        )
        SpikeReport.measure(CRITERION, "input_focused", after.optBoolean("focused"))

        val entered = after.optString("value")
        if (entered.isBlank()) {
            SpikeReport.unavailable(CRITERION, "entered_text", "input_value_remained_empty")
        } else {
            SpikeReport.measure(CRITERION, "entered_text", entered)
        }
    }

    private fun focusInput(activity: SpikeWebViewActivity, instrumentation: android.app.Instrumentation) {
        runOnUi(instrumentation) {
            activity.web.evaluateJavascript("document.getElementById('q').focus();", null)
            activity.showKeyboard()
        }
    }

    private fun awaitKeyboard(activity: SpikeWebViewActivity, device: UiDevice): Boolean {
        val deadline = System.currentTimeMillis() + KEYBOARD_TIMEOUT_MS
        while (System.currentTimeMillis() < deadline) {
            if (activity.imeBottomInset > 0) {
                // 插入值到位后再等一次布局，否则量到的是键盘动画中途的几何。
                device.waitForIdle()
                Thread.sleep(SETTLE_MS)
                return activity.imeBottomInset > 0
            }
            Thread.sleep(POLL_MS)
        }
        return false
    }

    /**
     * 优先走输入法自己的 `commitText`，失败退到无障碍 `ACTION_SET_TEXT`。
     *
     * 两条路都是真实的文字输入通道，但含义不同：前者经 IME 的 `InputConnection`，
     * 后者绕过输入法。返回值写进报告，读者据此判断这次 PASS 覆盖到哪一层。
     */
    private fun commitChinese(
        activity: SpikeWebViewActivity,
        instrumentation: android.app.Instrumentation,
    ): String {
        var committed = false
        runOnUi(instrumentation) {
            val connection = activity.web.onCreateInputConnection(EditorInfo())
            committed = connection?.commitText(CHINESE, CHINESE.length) == true
        }
        if (committed) {
            instrumentation.waitForIdleSync()
            Thread.sleep(SETTLE_MS)
            return "input_connection_commit_text"
        }
        val device = UiDevice.getInstance(instrumentation)
        val field = device.findObject(By.clazz("android.widget.EditText"))
        if (field != null) {
            runCatching { field.text = CHINESE }
                .onSuccess {
                    Thread.sleep(SETTLE_MS)
                    return "accessibility_action_set_text"
                }
        }
        return "no_channel_accepted_text"
    }

    private fun awaitProbe(
        activity: SpikeWebViewActivity,
        instrumentation: android.app.Instrumentation,
    ): JSONObject? {
        val deadline = System.currentTimeMillis() + PROBE_TIMEOUT_MS
        while (System.currentTimeMillis() < deadline) {
            val raw = evaluate(activity, instrumentation, "window.__yunjianProbe ? window.__yunjianProbe() : null")
            val decoded = decode(raw)
            if (decoded != null) {
                return decoded
            }
            Thread.sleep(POLL_MS)
        }
        return null
    }

    /**
     * `evaluateJavascript` 的结果是一段 **JSON 编码后的字符串**，所以 `__yunjianProbe`
     * 返回的 JSON 会被再包一层引号并转义。先按 JSON 字符串解一层，再按对象解析。
     */
    private fun decode(raw: String?): JSONObject? {
        if (raw == null || raw == "null") {
            return null
        }
        val unwrapped = runCatching { JSONObject("{\"v\":$raw}").getString("v") }.getOrNull() ?: raw
        return runCatching { JSONObject(unwrapped) }.getOrNull()
    }

    private fun evaluate(
        activity: SpikeWebViewActivity,
        instrumentation: android.app.Instrumentation,
        script: String,
    ): String? {
        var out: String? = null
        val latch = CountDownLatch(1)
        instrumentation.runOnMainSync {
            activity.web.evaluateJavascript(script) { value ->
                out = value
                latch.countDown()
            }
        }
        latch.await(PROBE_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        return out
    }

    private fun runOnUi(instrumentation: android.app.Instrumentation, block: () -> Unit) {
        instrumentation.runOnMainSync(block)
        instrumentation.waitForIdleSync()
    }

    private fun unavailableAll(reason: String) {
        for (key in listOf("entered_text", "keyboard_overlap_px", "input_visible", "visual_viewport_updated")) {
            SpikeReport.unavailable(CRITERION, key, reason)
        }
    }

    private companion object {
        const val CRITERION = "chinese_ime"

        /** 「云笺」两字：判据要的是中文提交成功，用产品自己的名字最直观。 */
        const val CHINESE = "云笺"
        const val KEYBOARD_TIMEOUT_MS = 12_000L
        const val PROBE_TIMEOUT_MS = 15_000L
        const val POLL_MS = 250L
        const val SETTLE_MS = 800L
    }
}
