/**
 * 外壳骨架。
 *
 * 标题栏来自 todo 60，检索与阅读来自 todo 61，设置来自 todo 62，背诵来自 todo 63。
 *
 * 路由刻意是一个多态的 `view` 而不是引入路由库：一个 URL 路由器要连带解决深链、返回栈与
 * Tauri 下的 history 行为，那些问题在一个只有几屏的桌面外壳里没有对应的用户需求。
 *
 * # `view` 从四态收成三态
 *
 * 设置改成了模态弹窗（`shell/SettingsDialog.tsx`），于是它**不再是一个 `view`**：
 * 弹窗打开时底下那一屏仍然挂载着，`view` 不变，关掉就回到原处。若继续把设置放在 `view` 里，
 * 「关掉弹窗回到刚才那屏」就得额外记一个「上一屏是谁」，而那个状态与真实语义不符——
 * 弹窗从来没有离开过任何一屏。
 *
 * # 导航为什么搬进 `shell/Sidebar.tsx`，而不是留在这里
 *
 * 原先的横排导航是内联样式直接写在本文件里的。改成侧栏之后它有了自己的状态（折叠）、
 * 自己的选中态规则（检索吸收阅读页）与自己的无障碍语义（`aria-current` 对
 * `aria-haspopup="dialog"`），这些都需要独立的断言。留在这里就只能靠 `App.test.tsx`
 * 从整棵树出发去验，那组断言会同时承担「导航对不对」与「屏接没接上」两件事。
 *
 * 但**入口的 testid 仍然由 `App.test.tsx` 从 `<App />` 出发去点**：那一组存在的理由是一次
 * 真实缺口（设置 14 个文件全实现、37 条断言全绿，页面上却没有入口能到它），
 * 组件级断言绕不过这一段。
 */
import { useMemo, useState } from "react";
import TitleBar from "./chrome/TitleBar";
import { useWindowChrome } from "./chrome/useWindowChrome";
import PoemDetailScreen from "./poem/PoemDetailScreen";
import ReciteScreen from "./recite/ReciteScreen";
import SearchScreen from "./search/SearchScreen";
import Sidebar, { type ShellSection } from "./shell/Sidebar";
import SettingsDialog from "./shell/SettingsDialog";
import DictionaryPanel from "./shell/DictionaryPanel";
import { SAMPLE_MODE_NOTICE, createSamplePorts } from "./data/samplePorts";
import { createTauriPorts } from "./data/tauriPorts";
import { createSampleSettingsPorts, createTauriSettingsPorts } from "./data/sampleSettingsPorts";
import { createSampleRecitePorts, createTauriRecitePorts } from "./data/sampleRecitePorts";
import { createSampleVoicePort, createTauriVoicePort } from "./data/sampleVoicePorts";

type View =
  | { kind: "search" }
  | { kind: "poem"; poemId: string }
  | { kind: "dictionary" }
  | { kind: "recite" };

/** 侧栏的选中态：阅读页归「检索」那一支，它是从检索进去的。 */
function sectionOf(view: View): ShellSection {
  if (view.kind === "recite" || view.kind === "dictionary") {
    return view.kind;
  }
  return "search";
}

export default function App() {
  const chrome = useWindowChrome();
  const [view, setView] = useState<View>({ kind: "search" });
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

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

  // 背诵端口同理：复习队列的 effect 以 port 为依赖，每帧重建会变成一个每帧重跑的
  // `due` + `stats`，而那两个在真实路径上要开 SQLite。
  const recitePorts = useMemo(() => createTauriRecitePorts() ?? createSampleRecitePorts(), []);

  // 语音端口同理：`VoicePanel` 的可用性探测 effect 以 port 为依赖，每帧重建会变成一个每帧
  // 重跑的 `voice_availability`，而那一条在真实路径上要枚举音频设备。
  const voicePort = useMemo(() => createTauriVoicePort() ?? createSampleVoicePort(), []);

  return (
    <div className="grid h-full grid-rows-[var(--titlebar-height)_1fr]">
      <TitleBar title="云笺" chrome={chrome} />
      {/* `min-h-0` 不可省：网格项的默认 `min-height: auto` 会让下面那个滚动容器
          按内容高度撑开，于是整页出现第二根滚动条而内容区自己不滚。 */}
      <div className="grid min-h-0 grid-cols-[auto_1fr]">
        <Sidebar
          section={sectionOf(view)}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={() => {
            setSidebarCollapsed((collapsed) => !collapsed);
          }}
          onSelect={(section) => {
            setView({ kind: section });
          }}
          settingsOpen={settingsOpen}
          onOpenSettings={() => {
            setSettingsOpen(true);
          }}
        />
        <main className="min-h-0 overflow-y-auto">
          {sample && (
            <p
              data-testid="sample-mode-notice"
              // 底色与下边框铺满整宽（它是一条全局状态横幅），但**文字与内容区共用
              // 同一条对齐基准**：`46rem` 与检索页、阅读页、设置页三处的 `max-width` 相同。
              // 不这么做的话，横幅文字贴左边缘而下方内容居中，两个对齐基准会同时出现在一屏上。
              className="m-0 border-b border-[var(--color-error-border)] bg-[var(--color-error-surface)] px-6 py-3 text-xs leading-[1.7] text-[var(--color-error-text)]">
              <span className="mx-auto block max-w-[46rem]">{SAMPLE_MODE_NOTICE}</span>
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
          {view.kind === "recite" && <ReciteScreen ports={recitePorts} voicePort={voicePort} />}
          {view.kind === "dictionary" && <DictionaryPanel port={ports.dictionary} />}
        </main>
      </div>
      <SettingsDialog
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
        }}
        ports={settingsPorts}
      />
    </div>
  );
}
