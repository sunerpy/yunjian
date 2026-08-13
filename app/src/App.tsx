/**
 * 外壳骨架。
 *
 * 标题栏来自 todo 60，检索与阅读来自 todo 61。设置（62）与背诵（63）各自往内容区里挂。
 *
 * 路由刻意是一个两态的 `view` 而不是引入路由库：当前只有「检索」与「详情」两屏，
 * 一个 URL 路由器要连带解决深链、返回栈与 Tauri 下的 history 行为，那属于有了第三屏之后的事。
 */
import { useMemo, useState } from "react";
import TitleBar from "./chrome/TitleBar";
import { useWindowChrome } from "./chrome/useWindowChrome";
import PoemDetailScreen from "./poem/PoemDetailScreen";
import SearchScreen from "./search/SearchScreen";
import { SAMPLE_MODE_NOTICE, createSamplePorts } from "./data/samplePorts";
import { createTauriPorts } from "./data/tauriPorts";

type View = { kind: "search" } | { kind: "poem"; poemId: string };

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

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "var(--titlebar-height) 1fr",
        height: "100%",
      }}>
      <TitleBar title="云笺" chrome={chrome} />
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
        {view.kind === "search" ? (
          <SearchScreen
            port={ports.search}
            onOpen={(poemId) => {
              setView({ kind: "poem", poemId });
            }}
          />
        ) : (
          <PoemDetailScreen
            poemId={view.poemId}
            poemPort={ports.poem}
            appreciationPort={ports.appreciation}
            onBack={() => {
              setView({ kind: "search" });
            }}
          />
        )}
      </main>
    </div>
  );
}
