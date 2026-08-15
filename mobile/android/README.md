# Android 工程落点

当前 binding verdict 为 `undetermined`，本目录不含伪造的 Kotlin/Compose 或 Tauri 工程。
若裁决变为 `tauri_mobile`，共享 React UI 的生成工程位于
`crates/yunjian-app/gen/android`；若裁决变为 `uniffi_native`，Jetpack Compose 工程落在本目录，
并且只能调用 `yunjian-mobile` 生成的绑定。

分发只接受 `arm64-v8a`、`armeabi-v7a`、`x86`、`x86_64` 四个 split APK 和一个 AAB；
universal APK 不作为默认产物。首个 Play upload 必须在 Play Console 的 internal testing track
手工完成，后续自动化才有可复用的应用与签名身份。
