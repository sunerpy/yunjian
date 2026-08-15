package top.onethinker.yunjian.spike

import android.app.Activity
import android.graphics.Rect
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import android.webkit.WebView
import android.widget.FrameLayout

/**
 * 判据③的被测界面：一个**边到边**窗口里贴底放置的 WebView 检索框。
 *
 * 这个 Activity 刻意放进**应用** APK 而不是测试 APK。判据③量的是「`targetSdk 35` 下
 * 键盘是否遮挡输入框」，而 edge-to-edge 的强制行为是**按包的 targetSdk** 生效的；
 * 若界面跑在测试包里，量到的就是测试包的 targetSdk 行为，与判据要问的不是同一件事。
 *
 * 输入框贴底是刻意的最坏情形：居中放置时键盘弹出后即使什么都不做也不会遮挡，
 * 那样的「PASS」证明不了任何事。
 */
class SpikeWebViewActivity : Activity() {

    lateinit var web: WebView
        private set

    /** 最近一次 IME 插入高度（设备像素）。0 表示软键盘未显示。 */
    @Volatile
    var imeBottomInset: Int = 0
        private set

    /** 最近一次系统栏底部插入高度；用于证明内容真的画到了系统栏之下。 */
    @Volatile
    var systemBarsBottomInset: Int = 0
        private set

    private lateinit var root: FrameLayout

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // 边到边。API 30+ 的这一句等价于 targetSdk 35 起系统强制施加的行为，
        // 显式写出来是为了让「本次确实在 edge-to-edge 下测量」可被复核。
        window.setDecorFitsSystemWindows(false)

        root = FrameLayout(this)
        web = WebView(this)
        WebView.setWebContentsDebuggingEnabled(true)
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        root.addView(
            web,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT,
                Gravity.BOTTOM,
            ),
        )
        setContentView(root)

        root.setOnApplyWindowInsetsListener { view, insets ->
            val ime = insets.getInsets(WindowInsets.Type.ime())
            val bars = insets.getInsets(WindowInsets.Type.systemBars())
            imeBottomInset = ime.bottom
            systemBarsBottomInset = bars.bottom
            // 键盘弹出时把容器底部顶起来。这一步就是判据③要检验的产品责任：
            // 边到边窗口里系统不再替应用避让键盘，不自己处理 ime 插入就一定会遮挡。
            view.setPadding(bars.left, bars.top, bars.right, maxOf(ime.bottom, bars.bottom))
            insets
        }

        web.loadDataWithBaseURL(BASE_URL, PAGE, "text/html", "utf-8", null)
    }

    /** 请求焦点并弹出软键盘。必须在 UI 线程调用。 */
    fun showKeyboard() {
        web.requestFocus()
        val manager = getSystemService(InputMethodManager::class.java)
        manager?.showSoftInput(web, 0)
    }

    /** 当前几何快照。所有长度都是设备像素。 */
    fun geometry(): Geometry {
        val bounds: Rect = windowManager.currentWindowMetrics.bounds
        val location = IntArray(2)
        web.getLocationOnScreen(location)
        val rootLocation = IntArray(2)
        root.getLocationOnScreen(rootLocation)
        return Geometry(
            displayHeight = bounds.height(),
            displayWidth = bounds.width(),
            rootTop = rootLocation[1],
            rootHeight = root.height,
            webTop = location[1],
            webHeight = web.height,
            imeBottomInset = imeBottomInset,
            systemBarsBottomInset = systemBarsBottomInset,
        )
    }

    data class Geometry(
        val displayHeight: Int,
        val displayWidth: Int,
        val rootTop: Int,
        val rootHeight: Int,
        val webTop: Int,
        val webHeight: Int,
        val imeBottomInset: Int,
        val systemBarsBottomInset: Int,
    ) {
        /**
         * 内容是否真的铺满整个显示区域并画到系统栏之下。
         *
         * 只看 `setDecorFitsSystemWindows(false)` 调用过是不够的——那是配置，不是结果。
         * 这里比的是「根视图顶边在屏幕 0 处、高度等于显示高度」且系统栏确实存在插入值。
         */
        val edgeToEdge: Boolean
            get() = rootTop == 0 && rootHeight >= displayHeight - 1 && systemBarsBottomInset > 0

        /** 软键盘顶边在屏幕坐标里的位置。 */
        val keyboardTop: Int
            get() = displayHeight - imeBottomInset
    }

    private companion object {
        const val BASE_URL = "https://yunjian.invalid/"

        /**
         * `viewport-fit=cover` + 贴底输入框，是 edge-to-edge 下最容易被键盘吃掉的布局。
         * `__yunjianViewport` 记录 `visualViewport.height` 的历次取值，让宿主侧能断言
         * 它**确实随键盘更新过**，而不是只读到一个最终值就假定更新发生了。
         */
        const val PAGE = """
<!doctype html>
<html lang="zh-CN"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<style>
  html,body{margin:0;padding:0;height:100%;font-family:sans-serif}
  #shell{position:fixed;left:0;right:0;top:0;bottom:0;display:flex;flex-direction:column;justify-content:flex-end}
  #q{width:100%;box-sizing:border-box;font-size:20px;padding:14px;border:2px solid #333}
</style></head>
<body><div id="shell"><input id="q" type="search" autocomplete="off" placeholder="检索"></div>
<script>
  window.__yunjianViewport = [];
  function snapshot() {
    var v = window.visualViewport;
    window.__yunjianViewport.push(v ? Math.round(v.height) : -1);
  }
  snapshot();
  if (window.visualViewport) {
    window.visualViewport.addEventListener('resize', snapshot);
    window.visualViewport.addEventListener('scroll', snapshot);
  }
  window.__yunjianProbe = function () {
    var input = document.getElementById('q');
    var rect = input.getBoundingClientRect();
    var ratio = window.devicePixelRatio || 1;
    return JSON.stringify({
      value: input.value,
      focused: document.activeElement === input,
      ratio: ratio,
      topPx: Math.round(rect.top * ratio),
      bottomPx: Math.round(rect.bottom * ratio),
      viewportHeight: window.visualViewport ? Math.round(window.visualViewport.height) : -1,
      viewportSamples: window.__yunjianViewport.slice(),
      hasVisualViewport: !!window.visualViewport
    });
  };
</script></body></html>
"""
    }
}
