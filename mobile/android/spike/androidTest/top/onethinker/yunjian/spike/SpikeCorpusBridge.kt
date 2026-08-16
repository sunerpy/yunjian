package top.onethinker.yunjian.spike

/**
 * 判据②的生产路径桥。Kotlin 侧不实现下载、校验或解压——那三件事必须由
 * `yunjian_core::assets::AssetResolver` 完成，否则量到的是一条产品不会走的路径。
 *
 * `libyunjian_spike.so` 打进**应用** APK 的 jniLibs，而不是测试 APK：instrumentation
 * 跑在应用进程里，`System.loadLibrary` 搜的是应用的 nativeLibraryDir。
 */
object SpikeCorpusBridge {

    /** 加载是否成功。失败时判据②如实记成未执行，而不是让整个测试类崩掉。 */
    val available: Boolean =
        runCatching { System.loadLibrary("yunjian_spike") }.isSuccess

    /**
     * @param manifestUrl 空串表示走产品自己的默认发布地址（`AssetResolver::discover`）。
     * @param dataRoot 应用私有存储根目录；语料落到它下面的 `corpus/`。
     * @param budgetSeconds 旁观预算，**不是判据阈值**；超时会如实上报。
     * @return 判据②的测量 JSON。
     */
    external fun measureCorpus(manifestUrl: String, dataRoot: String, budgetSeconds: Long): String?
}
