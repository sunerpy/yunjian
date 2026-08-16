package top.onethinker.yunjian

import android.app.Application
import top.yunjian.mobile.YunjianAndroid

/**
 * 应用入口。**唯一职责**是在任何 Rust 调用之前把 application context 交给 JNI。
 *
 * # 为什么必须在这里，而不是在第一次用到门面时
 *
 * `yunjian-ai` 的 Android 钥匙串经 `ndk-context` 取 JavaVM 与 application context。
 * 那个库只借用不拥有，所以 Rust 侧把 `GlobalRef` 存进 `OnceLock` 保证活到进程结束；
 * 但**交出去这一步只能由 Kotlin 做**。PR #102 的 spike APK 在真机上以
 * `panicked at ndk-context/src/lib.rs:72:30: android context was not initialized`
 * → SIGABRT 崩掉，成因正是 Tauri 外壳没有为这件事提供时机。
 *
 * `onCreate` 是进程里最早、且保证在任何 Activity 与任何 instrumentation 测试方法之前
 * 运行的点。放到 `MainActivity.onCreate` 会漏掉「instrumentation 直接调门面、不起
 * Activity」这条路径，而验收里有若干断言正是那样跑的。
 *
 * 传 `this`（Application）而不是某个 Activity：Rust 侧包装层虽然也会转成 application
 * context，但从这里传 Activity 等于让一个会被销毁的对象出现在调用链上。
 */
class YunjianApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        YunjianAndroid.initialize(this)
    }
}
