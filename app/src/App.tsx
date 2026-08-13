/**
 * 外壳骨架。
 *
 * 标题栏来自 todo 60，检索与阅读来自 todo 61。设置（62）与背诵（63）各自往内容区里挂。
 *
 * 路由刻意是一个多态的 `view` 而不是引入路由库：一个 URL 路由器要连带解决深链、返回栈与
 * Tauri 下的 history 行为，那些问题在一个只有几屏的桌面外壳里没有对应的用户需求。
 *
 * # 导航条为什么在这里，而不在 `TitleBar` 里
 *
 * 入口做成 `<main>` 上方独立的一行，而不是往 `chrome/TitleBar.tsx` 里加按钮。两个理由：
 *
 * 1. 标题栏的拖动区用 `data-tauri-drag-region="deep"`，往里加交互元素要同时顾及注入脚本的
 *    排除名单；导航是应用内容，不是窗口控件，混进去会让那个组件同时负责两件事。
 * 2. `TitleBar` 是 todo 60 的交付物，而设置（62）与背诵（63）都要加入口。**把入口集中在
 *    `App.tsx` 一处，冲突面就只有这一个文件的这一段**，不会波及标题栏。
 *
 * 样式沿用本文件既有的内联写法（样例模式横幅就是这么写的），同样是为了把改动收在一处。
 */
import { useMemo, useState } from "react";
import TitleBar from "./chrome/TitleBar";
import { useWindowChrome } from "./chrome/useWindowChrome";
import PoemDetailScreen from "./poem/PoemDetailScreen";
import SearchScreen from "./search/SearchScreen";
import SettingsScreen from "./settings/SettingsScreen";
import { SAMPLE_MODE_NOTICE, createSamplePorts } from "./data/samplePorts";
import { createTauriPorts } from "./data/tauriPorts";
import { createSampleSettingsPorts, createTauriSettingsPorts } from "./data/sampleSettingsPorts";

type View = { kind: "search" } | { kind: "poem"; poemId: string } | { kind: "settings" };

/** 选中态不另设 `data-active`：读屏与测试共用 `aria-current` 这一个信号。 */
function NavButton({
  label,
  active,
  testId,
  onClick,
}: {
  label: string;
  active: boolean;
  testId: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      aria-current={active ? "page" : undefined}
      onClick={onClick}
      style={{
        padding: "var(--space-1) var(--space-3)",
        border: "1px solid var(--color-border)",
        borderRadius: "var(--radius-sm)",
        background: active ? "var(--color-surface-raised)" : "transparent",
        color: active ? "var(--color-text)" : "var(--color-text-muted)",
        fontFamily: "var(--font-sans)",
        fontSize: "var(--text-sm)",
      }}>
      {label}
    </button>
  );
}

export default function App() {
  const chrome = useWindowChrome();
  const [view, setView] = useState<View>({ kind: "search" });

  // 端口只解析一次。反复解析会在每次渲染时重建对象，而 `SearchScreen` 的 effect
  // 以 port 为依赖——那会变成一个每帧重跑的 `listTags`。
  const { ports, sample } = useMemo(() => {
    const tauri = createTauriPorts();
    if (tauri !== null) {
      return { ports: tauri, sample: false };
    }
    return { ports: createSamplePorts(), sample: true };
  }, []);

  // 设置端口与上面同一条理由：解析一次。设置面板的 effect 也以 port 为依赖。
  const settingsPorts = useMemo(
    () => createTauriSettingsPorts() ?? createSampleSettingsPorts(),
    [],
  );

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "var(--titlebar-height) auto 1fr",
        height: "100%",
      }}>
      <TitleBar title="云笺" chrome={chrome} />
      <nav
        aria-label="主导航"
        data-testid="app-nav"
        style={{
          display: "flex",
          gap: "var(--space-2)",
          padding: "var(--space-2) var(--space-6)",
          borderBottom: "1px solid var(--color-border)",
          background: "var(--color-surface)",
        }}>
        <NavButton
          label="检索"
          testId="nav-search"
          // 详情页也归「检索」这一支：它是从检索进去的，把它算作第三个并列项
          // 会让导航出现一个用户无法直接抵达的条目。
          active={view.kind === "search" || view.kind === "poem"}
          onClick={() => {
            setView({ kind: "search" });
          }}
        />
        <NavButton
          label="设置"
          testId="nav-settings"
          active={view.kind === "settings"}
          onClick={() => {
            setView({ kind: "settings" });
          }}
        />
      </nav>
      <main style={{ overflowY: "auto" }}>
        {sample && (
          <p
            data-testid="sample-mode-notice"
            style={{
              margin: 0,
              padding: "var(--space-3) var(--space-6)",
              background: "var(--color-error-surface)",
              borderBottom: "1px solid var(--color-error-border)",
              color: "var(--color-error-text)",
              fontFamily: "var(--font-sans)",
              fontSize: "var(--text-xs)",
              lineHeight: 1.7,
            }}>
            {SAMPLE_MODE_NOTICE}
          </p>
        )}
        {view.kind === "search" && (
          <SearchScreen
            port={ports.search}
            onOpen={(poemId) => {
              setView({ kind: "poem", poemId });
            }}
          />
        )}
        {view.kind === "poem" && (
          <PoemDetailScreen
            poemId={view.poemId}
            poemPort={ports.poem}
            appreciationPort={ports.appreciation}
            onBack={() => {
              setView({ kind: "search" });
            }}
          />
        )}
        {view.kind === "settings" && <SettingsScreen ports={settingsPorts} />}
      </main>
    </div>
  );
}
