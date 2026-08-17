import SwiftUI
import YunjianMobile

/// 进程入口。
///
/// # 与 Android 的对应关系
///
/// Android 分成两处：`YunjianApplication.onCreate` 先把 application context 交给 JNI
/// （`YunjianAndroid.initialize(this)`），`MainActivity.onCreate` 再建 repository、触发首启
/// 物化并 `setContent`。iOS 没有「把 Context 交给原生层」这一步——UniFFI 的 Swift 生成物直接
/// 链进同一个可执行文件，不需要 JNI 引导。**所以这里只有 Android 的第二半。**
/// 那不是遗漏：`YunjianAndroid.initialize` 存在的唯一理由是 `ndk-context` 需要 JVM 侧的
/// Context，而 iOS 侧的等价物是「没有这件事」。
///
/// # 为什么首启物化在这里触发而不是更早
///
/// 物化要下载 212 MiB 并解压数 GiB。放在更早的初始化里会让**每一次**进程启动都开始下载，
/// 而 iOS 对启动耗时同样有看门狗。放在场景出现时，界面能同时显示进度。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
@main
struct YunjianApp: App {
    /// 本进程唯一那份 ViewModel。
    ///
    /// 与 Android 同一理由：每份 repository 会惰性构造自己的 `NativeFacade`，两份门面同时
    /// 持有同一个 SQLite 文件时写入方报 `database is locked`。`YunjianRepository.shared`
    /// 已把这件事收在进程级；这里再把 ViewModel 也钉在 App 上，避免场景重建时多出一份
    /// 正在跑的物化。
    @StateObject private var viewModel = MainViewModel()

    var body: some Scene {
        WindowGroup {
            ContentView(viewModel: viewModel, modelDir: viewModel.modelDir)
                .onAppear {
                    // `materialize()` 自身必须幂等到「同一进程里只有一次真的在跑」。
                    // 场景重建、从后台被回收后返回、XCUITest 逐条测试各拉一次界面，都会走到
                    // 这里；幂等由 `MainViewModel.materialize()` 与 `Materialization` 两层守住。
                    viewModel.materialize()
                }
        }
    }
}
