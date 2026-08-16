# Android 产品工程

binding verdict 是 **`uniffi_native`**（`crates/yunjian-mobile/src/lib.rs` 的
`BINDING_VERDICT`，2026-08-16 由判据②的真机实测确定）。因此本目录是一个真实的
Jetpack Compose 工程，**唯一允许的 Rust 入口**是 `crates/yunjian-mobile/bindings/`
下 UniFFI 生成的 Kotlin 与那份 Android 初始化包装器。

`crates/yunjian-mobile/tests/architecture.rs` 用两条断言守住这件事：
工程文件齐备且 `YunjianApplication.onCreate` 真的先调
`YunjianAndroid.initialize(this)`；产品源码不得自己声明 `external fun`（那意味着
绕过生成物直接接 JNI）。

## 怎么构建

```sh
export ANDROID_HOME=$HOME/Android/Sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<版本>   # r26 或更高
cargo install cargo-ndk --locked                   # 一次即可

cd mobile/android
gradle :app:assembleDebug :app:assembleDebugAndroidTest
```

`preBuild` 挂了两个任务：`cargoNdkBuild` 交叉编译 `yunjian-mobile` 并把
`libyunjian_mobile.so` 与它的两个 `NEEDED` 依赖（`libsherpa-onnx-c-api.so`、
`libonnxruntime.so`）摆进 `jniLibs/<abi>/`；`copyLicenseAssets` 把
`packaging/licenses/` 整目录搬进 `assets/licenses/`。

两个可调项：

| 属性              | 默认        | 含义                                                     |
| ----------------- | ----------- | -------------------------------------------------------- |
| `-Pyunjian.voice` | `true`      | 是否编译 `native-voice`。**关掉即 MIT 构建**，见下       |
| `-Pyunjian.abis`  | `arm64-v8a` | 逗号分隔。发布走四个：`arm64-v8a,armeabi-v7a,x86,x86_64` |

## 许可边界

**默认构建整体按 GPL-3.0 条款提供。** 依据是可验证的事实：
`aarch64-linux-android` 的 `libsherpa-onnx-c-api.so` 有 50 个 `espeak_*` 导出符号，
而 espeak-ng 是 GPL-3.0；它是 `libyunjian_mobile.so` 的 `NEEDED` 依赖，随 APK 分发。
完整论述见仓库根 [`LICENSES.md`](../../LICENSES.md)。

要一份 MIT 构建用 `-Pyunjian.voice=false`。那时 `startAsr` 返回
「当前原生库未启用 native-voice」而不是静默降级，语音入口据此显示具体原因。

## 真机验收

十条断言在 `app/src/androidTest/.../FullAcceptanceTest.kt`，
判据在 `xtask/src/acceptance/mobile/full_criteria.rs`。**设备侧只报测量值，
PASS/FAIL 由宿主侧判定**——让被测物自己判等于把门禁搬进被测物内部。

```sh
cd mobile/android && gradle :app:assembleDebug :app:assembleDebugAndroidTest
# 打包 → 上传 Device Farm → 回收测量与截图 → 落盘报告
cargo run -p xtask -- acceptance --platform android --set full
```

回传日志放 `docs/reports/mobile-qa-android-measurements.log`，截图放
`docs/reports/mobile-qa/`。缺回传时十条保持 `NOT EXECUTED`——远端池可达证明的是
「有真机」，不是「真机上跑过云笺」。

## 分发

`arm64-v8a`、`armeabi-v7a`、`x86`、`x86_64` 四个 split APK 加一个 AAB；
universal APK 不作为默认产物。首个 Play upload 必须在 Play Console 的
internal testing track 手工完成，后续自动化才有可复用的应用与签名身份。

## 已知的两个坑

**`assemble<变体>AndroidTest` 与应用 APK 必须同签名。** debug 变体天然一致，
所以验收走 debug。Device Farm 对**公共**设备一律重签
（`skipAppResign` 只对私有设备有效），因此设备侧脚本会卸掉重签的那份、改装
测试包里我们自己的两个 APK——构建期那道「同签名」断言拦不住它，重签发生在上传之后。

**`android.builder.sdkDownload=false` 是刻意的。** spike 那次 AGP 自动下载了
android-36，`targetSdk` 悄悄从 35 漂到 36，于是 edge-to-edge 语义跟着变了。
缺平台应该失败并指名缺哪一个，而不是自己补上一个别的。
