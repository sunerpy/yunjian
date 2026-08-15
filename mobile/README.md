# 移动端交付边界

`docs/reports/mobile-spike.json` 当前裁决是 `undetermined`，所以这里不生成 Tauri mobile 或
UniFFI 原生外壳。提前生成任何一支都会违反「实测后只实现一支」的门禁。

共享 React UI 在 `app/`，六组功能路由由 `app/src/mobileRoutes.ts` 声明并从 `<App />` 做
真实可达性测试；领域调用的唯一移动边界是 `crates/yunjian-mobile`。选型确定后，平台工程分别
落在 `mobile/android/` 与 `mobile/ios/`，或由 Tauri 生成到配置声明的 `gen/` 目录。

分发目标、ABI、上限和确切构建命令见 `distribution.toml`。真实产物生成后运行：

```bash
cargo run -p xtask -- mobile-distribution --artifacts-dir mobile/artifacts
```

命令扫描 APK/AAB/IPA 的 ZIP 目录，拒绝 corpus `.db`、语音模型和默认 universal APK，并把
实测字节数写入 `docs/reports/mobile-size.{md,json}`。没有产物或物理设备时对应项是
`NOT EXECUTED`，不是零字节或 PASS。
