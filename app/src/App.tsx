/**
 * 外壳骨架。
 *
 * 本 todo 只搭骨架：一条留给自绘标题栏（todo 60）的固定高度插槽，加一个内容区。
 * 检索与阅读（todo 61）、设置（62）、背诵（63）各自往内容区里挂路由。
 *
 * 刻意不在这里放任何交互控件。此刻放下的每个按钮都会在那三个 todo 里被重写一遍，
 * 而重写留下的死代码比空白难清。
 */
export default function App() {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "var(--titlebar-height) 1fr",
        height: "100%",
      }}>
      {/* 标题栏插槽。todo 60 会用 TitleBar 组件替换它，高度由同一个令牌决定，
          所以替换不会让下方内容跳动。 */}
      <header
        style={{
          background: "var(--color-titlebar-bg)",
          borderBottom: "1px solid var(--color-border)",
        }}
      />
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
