# Android 冒烟判据的 instrumented 测试

`xtask acceptance --platform android --set spike` 的四条判据里，①②③ 的必需测量值
（`rms`、`sha256_verified`、`entered_text` 等）**只能由跑在设备上的应用进程报出来**，
宿主侧的 `adb` 拿不到。这个目录就是那三个测试类。

## 目录

| 路径                                                        | 打进哪个 APK | 作用                                             |
| ----------------------------------------------------------- | ------------ | ------------------------------------------------ |
| `main/AndroidManifest.xml`                                  | 应用         | 声明 `SpikeWebViewActivity`                      |
| `main/top/onethinker/yunjian/spike/SpikeWebViewActivity.kt` | 应用         | 判据③的边到边 WebView 检索框                     |
| `androidTest/AndroidManifest.xml`                           | 测试         | 最小 manifest                                    |
| `androidTest/.../SpikeReport.kt`                            | 测试         | 测量行的唯一出口（文件 + logcat 三通道）         |
| `androidTest/.../SpikeCorpusBridge.kt`                      | 测试         | `libyunjian_spike.so` 的 JNI 声明                |
| `androidTest/.../SpikeMicrophoneTest.kt`                    | 测试         | 判据①                                            |
| `androidTest/.../SpikeCorpusTest.kt`                        | 测试         | 判据②                                            |
| `androidTest/.../SpikeImeTest.kt`                           | 测试         | 判据③                                            |
| `spike-androidtest.gradle.kts`                              | —            | 追加到 `gen/android/app/build.gradle.kts` 的片段 |

## 为什么测试不启动云笺主界面

真机实测（PR #102）：`top.onethinker.yunjian/.MainActivity` 一启动就 `SIGABRT`——
`ndk-context` 在没有 Android context 时是 `panic!` 而不是 `Err`，于是
`AppState::new` 首行 `KeyStore::open` 那条本该优雅降级到 `session_memory` 的分支
没机会执行。

这三个判据问的是「**设备与平台能否支撑这些能力**」，用来决定 `tauri_mobile` 还是
`uniffi_native`；它们不问「云笺的完整启动流程是否正常」——那是 todo 71 的产品验收。
所以测试刻意**不依赖主界面**：判据①直接用 `AudioRecord`，判据②直接调
`yunjian_core::assets::AssetResolver`，判据③用一个独立的 spike Activity。

那个启动 panic **没有在本轮修**：绕过它需要 `catch_unwind`，而那会让密钥静默退化成
仅本进程内存——用户可见的凭据存储行为变化，属安全边界，该由「移动端 API Key 存哪里」
这个决定来定，不该由一个门禁任务单方面决定。它也不会让任何判据变 PASS。

## 判据到测量值的对应

| 判据                       | 由哪个类报                          | 关键测量值                                                                                      |
| -------------------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------- |
| ① `microphone_capture`     | `SpikeMicrophoneTest`               | `sample_rate_hz`、`channel_count`、`rms`、`permission_plugin`                                   |
| ② `corpus_materialization` | `SpikeCorpusTest` → `yunjian-spike` | `artifact_bytes`、`sha256_verified`、`duration_seconds`、`atomic_install`、`crashed`            |
| ③ `chinese_ime`            | `SpikeImeTest`                      | `target_sdk`、`entered_text`、`keyboard_overlap_px`、`input_visible`、`visual_viewport_updated` |

④ `ios_testflight_submission` 不在这里：用户已决定不做商店提交，它保持 `NOT EXECUTED`。

## 两处刻意的边界

- **判据①测的是权限路径，不是 todo 46 的插件类。** 那个类是 Tauri 插件，要由 Rust 侧
  `tauri::plugin` 注册才存在，而那份注册属于 todo 69 的 `tauri_mobile` 分支——也就是本
  门禁要决定的事。让判据①依赖它就成了循环。所以测它的可观测契约：两条权限在包内声明
  齐全且 `RECORD_AUDIO` 运行期已授予。`permission_plugin_class_present=false` 如实记录。
- **Device Farm 没有音频注入。** 非零 `rms` 只能来自机架环境噪声。判据①要的正是「不是
  静音」，机架噪声足以证明；但它**不蕴含** todo 71 的
  `voice_recitation_round_succeeds_end_to_end`——那条要识别一段**已知**背诵，而已知音频
  喂不进这台设备。
