package top.onethinker.yunjian.spike

import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.json.JSONObject
import org.junit.Test
import org.junit.runner.RunWith

/**
 * 判据②：走**生产用的同一条**下载路径把发布语料物化进应用私有存储。
 *
 * Kotlin 侧只负责调用与转发。下载、SHA-256 校验、原子解压全部由
 * `yunjian_core::assets::AssetResolver::sync` 完成——方案点名要求走生产路径，
 * 因为一个测「随包资产」的门禁会用一条产品永远不会走的路径来决定框架选型。
 *
 * 阈值判定不在这里。这个类不认识「60 秒」，它只把实测值报出去。
 */
@RunWith(AndroidJUnit4::class)
class SpikeCorpusTest {

    @Test
    fun materializes_the_release_corpus_over_the_production_path() {
        SpikeReport.identify(CRITERION)
        val context = InstrumentationRegistry.getInstrumentation().targetContext

        if (!SpikeCorpusBridge.available) {
            unavailableAll("libyunjian_spike_not_loadable")
            return
        }
        val dataRoot = File(context.filesDir, "spike-corpus")
        SpikeReport.measure(CRITERION, "data_root", dataRoot.absolutePath)
        SpikeReport.measure(CRITERION, "free_bytes", File(context.filesDir.absolutePath).usableSpace)
        SpikeReport.measure(CRITERION, "supported_abis", Build.SUPPORTED_ABIS.joinToString("/"))

        val json = runCatching {
            SpikeCorpusBridge.measureCorpus("", dataRoot.absolutePath, BUDGET_SECONDS)
        }.onFailure { SpikeReport.note("生产路径桥抛出：${it.message}") }.getOrNull()
        if (json.isNullOrBlank()) {
            unavailableAll("bridge_returned_no_measurement")
            return
        }
        SpikeReport.note("corpus bridge json=$json")

        val measured = runCatching { JSONObject(json) }.getOrNull()
        if (measured == null) {
            unavailableAll("bridge_measurement_not_json")
            return
        }
        forward(measured)
    }

    /**
     * 把桥返回的键逐个转成测量行。
     *
     * `null` 值必须走 `unavailable` 而不是写成空串：生产路径失败时耗时确实**不存在**，
     * 而空串会被宿主侧判成「测到了一个空值」，进而记 FAIL——把一次未到达说成产品失败。
     */
    private fun forward(measured: JSONObject) {
        for (key in FORWARDED) {
            if (measured.isNull(key)) {
                val reason = measured.optString("failure").ifBlank { "production_path_did_not_report_$key" }
                SpikeReport.unavailable(CRITERION, key, reason.replace(' ', '_'))
            } else {
                SpikeReport.measure(CRITERION, key, measured.get(key))
            }
        }
    }

    private fun unavailableAll(reason: String) {
        for (key in FORWARDED) {
            SpikeReport.unavailable(CRITERION, key, reason)
        }
    }

    private companion object {
        const val CRITERION = "corpus_materialization"

        /** 旁观预算，不是判据阈值。给足是为了让超阈值也能被量到，而不是超时就消失。 */
        const val BUDGET_SECONDS = 600L

        /**
         * 前六项是判据②预声明的必需测量值，其余是解读它们所需的旁证：
         * 没有 `download_verify_seconds` / `decompress_seconds`，一次超阈值只能看到总耗时，
         * 无法判断是网络慢还是解压慢，而这两者对选型的含义完全不同。
         */
        val FORWARDED = listOf(
            "artifact_bytes",
            "sha256_verified",
            "duration_seconds",
            "atomic_install",
            "crashed",
            "production_path",
            "sha256",
            "download_verify_seconds",
            "decompress_seconds",
            "corpus_bytes",
            "poll_interval_ms",
            "derive_awaited",
            "started_from_clean_state",
            "residual_temp_files",
            "manifest_url",
        )
    }
}
