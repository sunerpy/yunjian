/**
 * Tauri IPC 背后的端口实现，与非 Tauri 宿主下的样例端口。
 *
 * # 两个宿主，两条路径，界面必须知道自己在哪一条上
 *
 * 命令本体属 todo 64。本模块只定义那一层的**调用形状**并在拿不到宿主时诚实降级：
 * 沿用 `chrome/windowControls.ts` 已证明可行的做法——`invoke` 拿不到就返回 `null`，
 * 由调用方决定怎么办，而不是返回一个「方法都在但什么也不做」的假实现
 * （那会让 UI 看起来是好的，而那正是最坏的形态）。
 *
 * # 样例数据必须自报身份
 *
 * 非 Tauri 宿主（`vite dev` 开在标签页、Vitest、Playwright）下走 `createSamplePorts`。
 * 它带的是**样例数据而不是语料**，因此界面上有一条常驻横幅说明这件事。
 * 不说的话，一个开发者截的图会被当成产品行为，而里面每一首诗的归属都是我编的。
 */

import { invoke } from "@tauri-apps/api/core";
import type { AppreciationState } from "../contracts/ai";
import type {
  MetaPage,
  PoemAnnotation,
  PoemDetail,
  SearchPage,
  TagSummary,
} from "../contracts/core";
import type {
  AppreciationPort,
  PoemAnnotationRequest,
  PoemDetailRequest,
  PoemPort,
  SearchPort,
  TagBrowseRequest,
  TextSearchRequest,
} from "./ports";

/**
 * todo 64 要注册的命令名。
 *
 * 集中在一处而不是散在调用点：命令名写错是**静默失败**（`invoke` 的 promise 被拒，
 * 界面只看到一条「检索失败」），所以它必须是一个能被 grep 出来核对的清单。
 */
export const IPC_COMMANDS = {
  searchText: "search_text",
  browseByTag: "browse_by_tag",
  listTags: "list_tags",
  poemDetail: "poem_detail",
  poemAnnotations: "poem_annotations",
  appreciate: "appreciate_poem",
} as const;

function inTauri(): boolean {
  // 判据与 `getCurrentWindow()` 抛异常的原因同源：`__TAURI_INTERNALS__` 由 Rust 侧注入。
  // 直接试调 `invoke` 也能判，但那会为了探测发一条真实 IPC。
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Tauri 宿主下的检索与阅读端口；不在宿主里时返回 `null`。 */
export function createTauriPorts(): {
  search: SearchPort;
  poem: PoemPort;
  appreciation: AppreciationPort;
} | null {
  if (!inTauri()) {
    return null;
  }

  const search: SearchPort = {
    searchText: (request: TextSearchRequest) =>
      invoke<SearchPage>(IPC_COMMANDS.searchText, { request }),
    browseByTag: (request: TagBrowseRequest) =>
      invoke<MetaPage>(IPC_COMMANDS.browseByTag, { request }),
    listTags: () => invoke<TagSummary[]>(IPC_COMMANDS.listTags),
  };

  const poem: PoemPort = {
    poemDetail: (request: PoemDetailRequest) =>
      invoke<PoemDetail>(IPC_COMMANDS.poemDetail, { request }),
    poemAnnotations: (request: PoemAnnotationRequest) =>
      invoke<PoemAnnotation>(IPC_COMMANDS.poemAnnotations, { request }),
  };

  const appreciation: AppreciationPort = {
    appreciate: (request: PoemDetailRequest) =>
      invoke<AppreciationState>(IPC_COMMANDS.appreciate, { request }),
  };

  return { search, poem, appreciation };
}
