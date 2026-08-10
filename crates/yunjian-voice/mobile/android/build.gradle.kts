// 云笺 Android 插件工程的 Gradle 片段。由外壳工程（todo 66/69）合并进
// `src-tauri/gen/android/app/build.gradle.kts`。
//
// **minSdk = 26，不是 Tauri 默认的 24。** 这是方案里点名要解决的版本冲突：
// `cpal` 的 Android 后端通过 `ndk` crate 的 `api-level-26` 特性绑定 AAudio
// （`rodio 0.22.2` 内部的 cpal 0.17.3 与曾被固定的 cpal 0.18.1 都是这一条），
// 而 Tauri 文档写的 Android 最低是 7.0 / API 24。留在 24 的后果不是编译失败而是
// **在 24/25 设备上运行期崩溃**，因为 AAudio 的符号在那两个 API 级别不存在。
// 所以 26 是产品要求，记录在 docs/PLATFORM-REQUIREMENTS.zh.md。
//
// 链接器也必须写死 android26：`cargo` 用的 clang 三元组里带 API 级别，
// `aarch64-linux-android24-clang` 会链接到不含 AAudio 的 sysroot。

android {
    defaultConfig {
        minSdk = 26
    }
}

dependencies {
    // `ContextCompat.checkSelfPermission` 与 Tauri 插件基类都在这里。
    implementation("androidx.core:core-ktx:1.13.1")
}
