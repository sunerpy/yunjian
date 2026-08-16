// 云笺 Android 产品工程（UniFFI 原生外壳）。
//
// 这个工程**不是** spike。spike 走 `cargo tauri android init` 生成的临时工程，只为量三条
// 可行性判据；本工程是裁决落地之后的产品外壳，唯一允许的 Rust 入口是 `yunjian-mobile`
// 生成的 UniFFI 绑定（`crates/yunjian-mobile/bindings/`）。
//
// `repositoriesMode` 刻意用 `PREFER_SETTINGS` 而不是 `FAIL_ON_PROJECT_REPOS`：AGP 的若干
// 内部组件会在配置阶段声明自己的仓库，`FAIL_ON_PROJECT_REPOS` 会把那种声明变成构建失败，
// 而那条报错读起来与本工程毫无关系。

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.PREFER_SETTINGS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "yunjian-android"

include(":app")
