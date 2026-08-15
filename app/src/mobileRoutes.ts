/**
 * 移动端共享 React UI 的六组功能路由。
 *
 * 这里的 `route` 是产品路由契约，不是浏览器 URL：当前桌面外壳用 `App` 内的状态机导航，
 * 移动框架裁决仍是 `undetermined`，因此不能提前生成 Tauri 或 UniFFI 外壳。每条记录同时钉住
 * 用户实际点击的入口和最终出现的功能表面；未来选定的移动外壳必须逐条实现同一张表，而不能
 * 因换了导航框架静默丢掉 AI、语音或钥匙串。
 */

export const MOBILE_FEATURE_GROUPS = [
  "search",
  "detail",
  "ai",
  "recitation",
  "voice",
  "settings-keystore",
] as const;

export type MobileFeatureGroup = (typeof MOBILE_FEATURE_GROUPS)[number];

export interface MobileRoute {
  group: MobileFeatureGroup;
  route: string;
  parent: MobileFeatureGroup | null;
  entryTestId: string;
  surfaceTestId: string;
}

export const MOBILE_ROUTES: readonly MobileRoute[] = [
  {
    group: "search",
    route: "/search",
    parent: null,
    entryTestId: "nav-search",
    surfaceTestId: "search-input",
  },
  {
    group: "detail",
    route: "/poems/:poemId",
    parent: "search",
    entryTestId: "result-row",
    surfaceTestId: "poem-detail",
  },
  {
    group: "ai",
    route: "/poems/:poemId/ai",
    parent: "detail",
    entryTestId: "result-row",
    surfaceTestId: "ai-panel",
  },
  {
    group: "recitation",
    route: "/recite",
    parent: null,
    entryTestId: "nav-recite",
    surfaceTestId: "recite-screen",
  },
  {
    group: "voice",
    route: "/recite/voice",
    parent: "recitation",
    entryTestId: "mode-voice",
    surfaceTestId: "voice-availability-note",
  },
  {
    group: "settings-keystore",
    route: "/settings/keystore",
    parent: "search",
    entryTestId: "nav-settings",
    surfaceTestId: "key-storage-indicator",
  },
] as const;
