// 云笺 Android 产品工程的根构建脚本。
//
// 只声明插件版本，不做任何配置：模块级配置都在 `app/build.gradle.kts`，让「哪里改哪个」
// 只有一个答案。

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
}
