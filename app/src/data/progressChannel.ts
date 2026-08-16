/**
 * 长任务事件通道的**唯一**构造点。
 *
 * # 为什么要把 `new Channel` 收到一个函数里
 *
 * 一个 `tauri::ipc::Channel` 有两件事必须一起做：**建出来交给 `invoke`**，以及
 * **订阅 `onmessage`**。少了前者，命令在参数反序列化阶段就被拒（Tauri 报
 * `missing required key onEvent`，界面看到的是「功能坏了」）；少了后者，命令跑得通、
 * 事件也确实发过来了，**但全部被丢在地上**——这一种没有任何报错，只表现为「过程中界面
 * 什么都不说」。
 *
 * 本项目两种都栽过，而且是分两次栽的：
 *
 * 1. `fetch_corpus` 与 `appreciate_poem` 压根不建 Channel，两个功能在真机上直接调不通
 *    （PR #104 的真机验收逮到，PR #105 补齐）。
 * 2. 补齐之后 `fetch_corpus` **建了却不订阅**，注释还自陈「设置面板暂不渲染进度」，
 *    于是首启物化 474,043 首诗全程无反馈（本次修复）。
 *
 * 第二次的教训是：**建了通道 ≠ 读了通道。** 只要「建」和「订阅」还是调用点自己写的两行，
 * 就随时可以只写第一行。所以这里把两件事合成一个不可分的动作——
 * `onEvent` 是必需参数，函数体一定给 `onmessage` 赋值，调用点拿不到一个未订阅的通道。
 * 与 `contracts/settings.ts` 让密钥「在类型上无从表达」同一路：让错误写不出来，
 * 而不是指望后来者记得补第二行。
 */

import { Channel } from "@tauri-apps/api/core";

/**
 * 建一个已订阅的事件通道，交给 `invoke` 作命令参数。
 *
 * 参数名要与 Rust 侧形参逐字一致（`on_event` → `onEvent`），这一点由各端口的调用点负责；
 * 本函数只保证「交出去的通道一定有人在读」。
 */
export function progressChannel<E>(onEvent: (event: E) => void): Channel<E> {
  const channel = new Channel<E>();
  channel.onmessage = onEvent;
  return channel;
}
