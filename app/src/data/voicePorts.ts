/**
 * 语音跟读的数据访问端口。
 *
 * # 与 `data/recitePorts.ts` 同一条理由，但多一件事：这里有流
 *
 * 四个方法里有两个是**长任务**（会话、模型下载），它们的中间结果走
 * `tauri::ipc::Channel`，不是 event 也不是轮询。所以端口签名把 `onEvent` 作为一个
 * 显式回调参数，而不是让调用方自己去建一个 Channel——那会把「事件形状」这件事散到每个
 * 调用点，而 Channel 的名字写错是**静默失败**（`invoke` 的 promise 被拒，界面只看到一条
 * 「会话失败」）。
 *
 * 每个方法都对应**已经存在**的 Rust 命令，没有一个是新发明的能力：
 *
 * | 端口方法 | Rust 侧命令 | 出处 |
 * | --- | --- | --- |
 * | `availability` | `voice_availability` | `voice_ipc.rs` |
 * | `demonstrate` | `voice_demonstrate` | `voice_ipc.rs` |
 * | `startSession` | `voice_start_session` | `voice_ipc.rs` |
 * | `fetchModel` | `voice_fetch_model` | `voice_ipc.rs` |
 * | `cancel` | `cancel_operation` | `ipc.rs` |
 *
 * # 取消为什么是端口的一部分
 *
 * 模型下载动辄几百兆，一个不能取消的进度条等于把用户锁在那里。取消走的是与赏析流、
 * 语料派生完全同一条路（`cancel_operation` + `operation_id`），因此界面不需要为语音
 * 另学一套。**`operation_id` 由调用方给**并原样回显在落点里，于是「我取消的是哪一个」
 * 不依赖任何猜测。
 */

import type {
  ModelFetchEvent,
  VoiceAvailability,
  VoiceDemonstration,
  VoiceModelOutcome,
  VoiceOutcome,
  VoiceSessionEvent,
} from "../contracts/voice";

/** 会话请求。 */
export interface VoiceSessionRequest {
  poem_id: string;
  /**
   * 是否逐行先播示范音。
   *
   * 关掉即纯复诵，此时时长比失去基准并退为 1.0（`session.rs` 的 `SessionPlan.demonstrate`）。
   * **打开时播放与录音仍然绝不重叠**：那由会话状态机保证，不是由这个开关保证。
   */
  demonstrate: boolean;
  /** 取消用的标识；省略时由 Rust 侧生成并在落点里回显。 */
  operation_id?: string;
}

/** 取模型请求。 */
export interface VoiceFetchModelRequest {
  name: string;
  operation_id?: string;
}

/** 语音跟读需要的全部端口。 */
export interface VoicePort {
  /** 语音在本机可用不可用。**界面在渲染任何语音控件之前先问这一条。** */
  availability(): Promise<VoiceAvailability>;
  /** 合成一段示范朗读，返回可播放地址与逐音步时间戳。 */
  demonstrate(poemId: string): Promise<VoiceDemonstration>;
  /** 跑一次跟读会话，中间结果经 `onEvent` 逐条送出。 */
  startSession(
    request: VoiceSessionRequest,
    onEvent: (event: VoiceSessionEvent) => void,
  ): Promise<VoiceOutcome>;
  /** 按需取一个语音模型，进度经 `onEvent` 送出，可经 [`cancel`] 取消。 */
  fetchModel(
    request: VoiceFetchModelRequest,
    onEvent: (event: ModelFetchEvent) => void,
  ): Promise<VoiceModelOutcome>;
  /** 取消一个正在跑的长任务。返回是否命中了一个已登记的操作。 */
  cancel(operationId: string): Promise<boolean>;
}
