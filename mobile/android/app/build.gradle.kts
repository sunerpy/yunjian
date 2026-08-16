// 云笺 Android 应用模块。
//
// 三条硬约束，每条都对应一次真机上量到的事实：
//
// 1. **`minSdk = 26`。** todo 46 实测：cpal 的 Android 后端绑定 AAudio，其符号在 API 24/25
//    不存在。这不是保守取值，是能不能启动的分界。
//
// 2. **`targetSdk = 35` 写死。** spike 那次 AGP 自动下载了 android-36，`targetSdk` 悄悄漂到
//    36，于是 edge-to-edge 语义跟着变了（真机量到 `keyboard_overlap_px` 的那套行为按包的
//    targetSdk 生效）。`gradle.properties` 里已关掉 `sdkDownload`，这里再把值钉住。
//
// 3. **`jniLibs` 由 `cargoNdkBuild` 供给，不进版本库。** 四个 ABI 的 `.so` 合计数百 MiB；
//    签入它们会让仓库无法克隆。任务在 `preBuild` 前跑，缺 NDK 时**失败并指名缺什么**，
//    而不是产出一个装上去就 `UnsatisfiedLinkError` 的 APK。

import org.gradle.internal.os.OperatingSystem

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// 仓库根。`mobile/android/app` -> 上溯三级。写成 `rootDir.parentFile.parentFile` 而不是
// 相对字符串：后者在 `--project-dir` 下会解析到别的地方。
val repoRoot: File = rootDir.parentFile.parentFile

/** UniFFI 生成物与 Android 初始化包装器的所在处。唯一允许的 Rust 入口。 */
val bindingsDir: File = File(repoRoot, "crates/yunjian-mobile/bindings")

/**
 * 是否连语音一起构建。
 *
 * 关掉时 `libyunjian_mobile.so` 不链接 sherpa-onnx，产物是 MIT；打开时预编译的
 * sherpa-onnx 静态含 GPL-3.0 的 espeak-ng（实测该 `.so` 有 50 个 `espeak_*` 导出符号），
 * 整个分发物须按 GPL-3.0 条款提供。见仓库根 `LICENSES.md`。
 *
 * **默认打开**：todo 69 明写「Voice ships on mobile in both branches」，移动产品要与桌面
 * 同等能力。关掉它只应发生在刻意生产一份 MIT 构建时。
 */
val withVoice: Boolean = (findProperty("yunjian.voice") as String? ?: "true").toBoolean()

/** 本次构建的 ABI。真机验收只需 arm64-v8a；发布走四个。 */
val targetAbis: List<String> =
    (findProperty("yunjian.abis") as String? ?: "arm64-v8a").split(",").map { it.trim() }

val abiToRustTarget = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
    "x86" to "i686-linux-android",
    "x86_64" to "x86_64-linux-android",
)

android {
    namespace = "top.onethinker.yunjian"
    compileSdk = 35

    defaultConfig {
        applicationId = "top.onethinker.yunjian"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters.addAll(targetAbis)
        }
    }

    signingConfigs {
        // 验收要求应用 APK 与 androidTest APK 同签名，否则 `am instrument` 报
        // `INSTALL_FAILED_SHARED_USER_INCOMPATIBLE`，那条报错读起来与测试内容无关。
        // debug 签名对两者天然一致，所以验收走 debug 变体。
        getByName("debug") {
            storeFile = File(System.getProperty("user.home"), ".android/debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        getByName("debug") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    sourceSets {
        getByName("main") {
            // 生成的 Kotlin 与 Android 初始化器直接进源集，不经复制：复制会产生一份可以与
            // 生成物漂移的副本，而漂移的表现是运行时 `UnsatisfiedLinkError`。
            kotlin.srcDir(File(bindingsDir, "generated"))
            kotlin.srcDir(File(bindingsDir, "android"))
            jniLibs.srcDir(layout.buildDirectory.dir("rustJniLibs"))
            // 许可原文随分发物走。开着 voice 的 APK 整体按 GPL-3.0 条款提供，而
            // 「源码可得」只满足一半——声明义务要求原文在分发物里。落到 assets/licenses/，
            // 与命令行归档带的是同一份 `packaging/licenses/`。
            assets.srcDir(layout.buildDirectory.dir("licenseAssets"))
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        // uiautomator 与 junit 都带 LICENSE 元数据；不排除会在打包阶段报重复文件，
        // 而那条错误读起来与本次改动毫无关系。
        resources.excludes.add("META-INF/LICENSE*")
        resources.excludes.add("META-INF/AL2.0")
        resources.excludes.add("META-INF/LGPL2.1")
        resources.excludes.add("META-INF/NOTICE*")
        jniLibs {
            // sherpa-onnx 与 onnxruntime 的 `.so` 必须能被 `dlopen`，压缩过的拿不到
            // 文件描述符。UniFFI 的 JNA 也要求 `.so` 以未压缩形态落在 nativeLibraryDir。
            useLegacyPackaging = false
        }
    }
}

/**
 * 交叉编译 `yunjian-mobile` 并把 `.so` 摆进 `jniLibs/<abi>/`。
 *
 * 用 `cargo-ndk` 而不是手写 `--target` + 一堆 `CC_*`/`AR_*` 环境变量：`ring` 与 `bzip2-sys`
 * 都要目标侧 C 编译器，那套变量名（`CC_aarch64_linux_android` 等）漂一个字母就报成
 * `ToolNotFound`，而那条错误不指向真因。`cargo-ndk` 按 NDK 布局把它们一并导出。
 *
 * 除主库外还要搬 `libsherpa-onnx-c-api.so` 与 `libonnxruntime.so`：它们是
 * `libyunjian_mobile.so` 的 `NEEDED` 依赖，缺任何一个都在 `System.loadLibrary` 处
 * 抛 `UnsatisfiedLinkError`，而报错文字只提主库的名字。
 */
val cargoNdkBuild by tasks.registering {
    group = "build"
    description = "用 cargo-ndk 交叉编译 yunjian-mobile 并把 .so 摆进 jniLibs"

    val outputDir = layout.buildDirectory.dir("rustJniLibs")
    outputs.dir(outputDir)
    inputs.dir(File(repoRoot, "crates/yunjian-mobile/src"))
    inputs.file(File(repoRoot, "crates/yunjian-mobile/Cargo.toml"))
    inputs.property("withVoice", withVoice)
    inputs.property("abis", targetAbis)

    doLast {
        val ndkHome = listOf("ANDROID_NDK_HOME", "NDK_HOME", "ANDROID_NDK_ROOT")
            .firstNotNullOfOrNull { System.getenv(it) }
            ?: throw GradleException(
                "缺少 Android NDK：请把 ANDROID_NDK_HOME 指向 NDK r26 或更高版本。" +
                    "yunjian-mobile 的 ring 与 bzip2-sys 依赖目标侧 C 编译器，没有 NDK 无法交叉编译。",
            )
        val cargoNdk = File(System.getProperty("user.home"), ".cargo/bin/cargo-ndk")
        if (!cargoNdk.isFile && OperatingSystem.current().isUnix) {
            throw GradleException(
                "找不到 cargo-ndk（${cargoNdk.absolutePath}）。装它：cargo install cargo-ndk --locked",
            )
        }

        val features = if (withVoice) "native-voice" else "uniffi"
        val jniRoot = outputDir.get().asFile
        jniRoot.mkdirs()

        val abiArgs = targetAbis.flatMap { listOf("-t", it) }
        providers.exec {
            workingDir = repoRoot
            environment("ANDROID_NDK_HOME", ndkHome)
            environment("NDK_HOME", ndkHome)
            commandLine(
                listOf("cargo", "ndk") + abiArgs +
                    listOf(
                        "--platform", "26",
                        "-o", jniRoot.absolutePath,
                        "build",
                        "-p", "yunjian-mobile",
                        "--features", features,
                        "--release",
                    ),
            )
        }.result.get().assertNormalExitValue()

        // cargo-ndk 只搬 cargo 认识的产物。sherpa-onnx 与 onnxruntime 是 sherpa-rs-sys
        // 下载的预编译件，落在 target/<triple>/release/ 下，得自己搬。
        if (withVoice) {
            for (abi in targetAbis) {
                val triple = abiToRustTarget[abi]
                    ?: throw GradleException("未知 ABI `$abi`；只接受 ${abiToRustTarget.keys}")
                val releaseDir = File(repoRoot, "target/$triple/release")
                val abiDir = File(jniRoot, abi).apply { mkdirs() }
                for (name in listOf("libsherpa-onnx-c-api.so", "libonnxruntime.so")) {
                    val source = File(releaseDir, name)
                    if (!source.isFile) {
                        throw GradleException(
                            "语音构建缺少 $name（找过 ${source.absolutePath}）。" +
                                "它是 libyunjian_mobile.so 的 NEEDED 依赖，缺它会在 " +
                                "System.loadLibrary 抛 UnsatisfiedLinkError，而报错只提主库的名字。",
                        )
                    }
                    source.copyTo(File(abiDir, name), overwrite = true)
                }
            }
        }

        for (abi in targetAbis) {
            val main = File(File(jniRoot, abi), "libyunjian_mobile.so")
            if (!main.isFile) {
                throw GradleException("cargo-ndk 没有为 $abi 产出 libyunjian_mobile.so")
            }
        }
    }
}

/**
 * 把 `packaging/licenses/` 整目录搬进 APK 的 `assets/licenses/`。
 *
 * **整目录拷贝，不逐个列文件。** 逐个列会在有人往那个目录加一份新许可时静默漏掉它，
 * 而漏掉的表现是分发物缺一份声明——那不是构建失败，是合规缺口。仓库侧
 * `cargo test -p yunjian-cli --test distribution_licenses` 已经守住那个目录本身的完整性。
 */
val copyLicenseAssets by tasks.registering(Copy::class) {
    group = "build"
    description = "把 packaging/licenses/ 搬进 APK 的 assets/licenses/"
    from(File(repoRoot, "packaging/licenses"))
    into(layout.buildDirectory.dir("licenseAssets/licenses"))
}

tasks.named("preBuild") {
    dependsOn(cargoNdkBuild, copyLicenseAssets)
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)

    // UniFFI 生成的 Kotlin 走 JNA 调 cdylib。`@aar` 带 JNA 自己的原生桥；
    // 换普通 jar 会在运行时报找不到 libjnidispatch.so。
    implementation("${libs.jna.get()}@aar")

    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.uiautomator)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
}
