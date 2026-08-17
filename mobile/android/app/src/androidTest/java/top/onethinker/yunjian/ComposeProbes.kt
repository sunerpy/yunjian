package top.onethinker.yunjian

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.compose.ui.test.onNodeWithTag

/**
 * 断言里反复要做的三件读操作。
 *
 * # 为什么 `textOrNull` 而不是 `assertTextEquals`
 *
 * 这些测试**不下结论**，只报测量值。`assertTextEquals` 在不匹配时抛异常，于是一次
 * 「文案与预期不同」会让整个测试方法中断，后面几项测量跟着一起丢——而那些测量正是
 * 宿主侧判断到底是产品坏了还是这一项没测到所需要的。读成 `null` 让缺失成为一个可上报
 * 的事实，而不是一次崩溃。
 */
fun ComposeTestRule.waitUntilNodeExists(tag: String, timeoutMs: Long) {
    waitUntil(timeoutMillis = timeoutMs) { exists(tag) }
}

fun SemanticsNodeInteractionsProvider.exists(tag: String): Boolean =
    readNodes(tag).isNotEmpty()

/**
 * 读一组节点，**容忍重组期间的瞬时失败**。
 *
 * # 为什么要容忍
 *
 * `fetchSemanticsNodes()` 会同步等待布局稳定，而语料物化期间界面每秒重组几十次；
 * 在那种状态下反复读会撞上
 * `IllegalArgumentException: performMeasureAndLayout called during measure layout`。
 * 真机实测这是**随时长累积的概率事件**：t02 每 500 ms 读一次、持续十几分钟，
 * 第十五轮与第二十三轮各撞上一次，把一次本该 PASS 的物化打成 NOT EXECUTED。
 *
 * 抛出时返回空表示「**本次没读到**」而不是「不存在」——调用方本来就在轮询，下一轮再读。
 * 这不会掩盖真的缺失：真缺失时每一轮都读不到，最终照样超时并带原因上报。
 */
private fun SemanticsNodeInteractionsProvider.readNodes(tag: String) =
    runCatching { onAllNodes(hasTestTag(tag)).fetchSemanticsNodes() }
        .getOrDefault(emptyList())

/**
 * 节点上的可读文本，缺节点时为 `null`。
 *
 * 先读 `EditableText`（`OutlinedTextField` 的当前值放在这里，`Text` 属性拿到的是 label），
 * 再退回 `Text`。只读后者会让「预填字段里有什么」永远量成 label 的文字。
 */
fun SemanticsNodeInteractionsProvider.textOrNull(tag: String): String? {
    val node = readNodes(tag).firstOrNull() ?: return null
    node.config.getOrNull(SemanticsProperties.EditableText)?.let { return it.text }
    return node.config.getOrNull(SemanticsProperties.Text)
        ?.joinToString(separator = "") { it.text }
}
