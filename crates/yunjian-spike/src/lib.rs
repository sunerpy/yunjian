//! 云笺 Android 冒烟判据②的生产路径桥。
//!
//! 只做一件事：在真机的应用进程里调用**生产用的**语料获取路径，并把判据②预声明的
//! 测量值以 JSON 交回 Kotlin 侧的 instrumented 测试。裁决不在这里做——本 crate 不认识
//! 「60 秒」这个阈值，阈值判定归 `xtask acceptance`。

pub mod corpus;

#[cfg(target_os = "android")]
mod android;
