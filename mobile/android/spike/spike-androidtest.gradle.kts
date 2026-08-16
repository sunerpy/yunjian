// 云笺 Android 冒烟判据的 Gradle 片段。由 `.aws/buildspec-android-spike.yml` 在
// `cargo tauri android init` 之后**追加**到 `gen/android/app/build.gradle.kts`。
//
// 只做四件事，每件都对应一条判据能否被执行：
//
// 1. **`targetSdk = 35`。** 判据③的阈值写的是 `targetSdk == 35`，而 PR #102 在真机上量到
//    36——那不是阈值错了，是构建从未把它钉住：tauri 模板取的是 `compileSdk` 的默认值，
//    AGP 又会自动下载缺失的平台，于是 36 就这么进来了。判据要求在 35 上测，就把它钉在 35。
//    edge-to-edge 的强制行为**按包的 targetSdk** 生效，所以这一行决定判据③测的是哪套语义。
//
// 2. **androidTest 源集与依赖。** 三个 instrumented 测试类在这里才成为可被
//    `am instrument` 调起的东西。runner 必须显式指定：tauri 的模板不含 androidTest 配置。
//
// 3. **把 spike Activity 编进应用 APK。** 它必须在应用包里，理由见
//    `SpikeWebViewActivity` 的类注释——判据③量的是应用自己 targetSdk 下的 edge-to-edge 行为。
//
// 4. **`useLegacyPackaging` 关掉压缩的例外。** 不动它：`libyunjian_spike.so` 走 jniLibs
//    的默认打包路径，`System.loadLibrary` 才找得到。这里写出来是为了记下「刻意不改」。

android {
    defaultConfig {
        // todo 46 实测的底线：cpal 的 Android 后端绑定 AAudio，其符号在 API 24/25 不存在。
        minSdk = 26
        targetSdk = 35
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    sourceSets {
        getByName("main") {
            java.srcDir("../../../../../mobile/android/spike/main")
        }
        getByName("androidTest") {
            java.srcDir("../../../../../mobile/android/spike/androidTest")
            manifest.srcFile("../../../../../mobile/android/spike/androidTest/AndroidManifest.xml")
        }
    }
    // 测试 APK 里会打进 uiautomator 与 junit，两者都带 LICENSE 元数据；不排除会在
    // 打包阶段报重复文件，而那条错误读起来与本次改动毫无关系。
    packaging {
        resources.excludes.add("META-INF/LICENSE*")
        resources.excludes.add("META-INF/AL2.0")
        resources.excludes.add("META-INF/LGPL2.1")
    }
}

dependencies {
    androidTestImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test:core:1.6.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test:rules:1.6.1")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
}
