import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
// 顺序有意义：Tailwind 先进，`styles.css` 后进。
// Tailwind 的产出全在 `@layer` 里，而 `styles.css` 无层——无层胜过有层，
// 于是既有手写规则不会被 preflight 或 utility 改掉。理由详见 `tailwind.css` 顶部。
import "./tailwind.css";
import "./styles.css";

// StrictMode 开着是刻意的：它会二次调用 effect，而自绘标题栏（todo 60）的
// maximize 订阅正是靠这一点暴露「卸载后仍写状态」的缺陷。关掉它等于把那个缺陷藏到真机上。
const container = document.getElementById("root");
if (!container) {
  throw new Error("找不到 #root 挂载点：index.html 与入口不一致");
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
