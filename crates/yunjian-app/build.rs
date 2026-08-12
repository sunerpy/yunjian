//! Tauri 的构建期代码生成：解析 `tauri.conf.json`（在 macOS 上再叠加
//! `tauri.macos.conf.json`）、内嵌前端资源与图标、生成 ACL 权限表。
//!
//! 刻意不做别的事。图标校验属 todo 65，权限扩充属 todo 60，两者都不该藏在 build script 里
//! ——build script 的失败诊断远比一条测试断言难读。

fn main() {
    tauri_build::build();
}
