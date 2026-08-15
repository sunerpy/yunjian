# iOS 工程落点

当前 binding verdict 为 `undetermined`，本目录不含伪造的 SwiftUI 或 Tauri 工程。
若裁决变为 `tauri_mobile`，共享 React UI 的生成工程位于
`crates/yunjian-app/gen/apple`；若裁决变为 `uniffi_native`，SwiftUI 工程落在本目录，并且只能
调用 `yunjian-mobile` 生成的绑定。

真实 archive 必须在装有 Xcode 26 / iOS 26 SDK、Distribution 证书和 provisioning profile
的 macOS runner 上产生，再上传 TestFlight；Linux 不能替代这一步。
