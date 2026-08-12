/**
 * 外壳骨架。
 *
 * 标题栏插槽由 todo 60 的 `TitleBar` 填上，高度仍取 `--titlebar-height`——与骨架时期
 * 同一个令牌，所以替换没有让下方内容跳动。检索与阅读（todo 61）、设置（62）、
 * 背诵（63）各自往内容区里挂路由。
 *
 * 刻意不在这里放任何交互控件。此刻放下的每个按钮都会在那三个 todo 里被重写一遍，
 * 而重写留下的死代码比空白难清。
 */
import TitleBar from "./chrome/TitleBar";
import { useWindowChrome } from "./chrome/useWindowChrome";

export default function App() {
  const chrome = useWindowChrome();

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "var(--titlebar-height) 1fr",
        height: "100%",
      }}>
      <TitleBar title="云笺" chrome={chrome} />
      <main
        style={{
          display: "grid",
          placeItems: "center",
          padding: "var(--space-8)",
        }}>
        <p
          style={{
            color: "var(--color-text-muted)",
            fontFamily: "var(--font-serif)",
            fontSize: "var(--text-lg)",
            margin: 0,
          }}>
          云笺
        </p>
      </main>
    </div>
  );
}
