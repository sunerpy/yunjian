/**
 * 自绘标题栏的呈现层。
 *
 * 给定 `useWindowChrome` 的返回值，这个组件是纯函数：它不发 IPC、不订阅、不持状态。
 * 因此下面每一条决定都能在 jsdom 里被验证，不需要真实窗口。
 *
 * # 三条「写错了不报错、只是行为不对」的规则
 *
 * 1. **拖动区用 `data-tauri-drag-region="deep"`，不是裸属性。** 裸属性的语义是
 *    `el === composedPath()[0]`，于是点在标题文字上不会拖动窗口，只有容器自己那几像素
 *    才会——用户看到的是「标题栏有的地方能拖、有的地方不能」。`deep` 会向下遍历子树，
 *    同时 Tauri 注入的脚本仍然排除 `A/BUTTON/INPUT/SELECT/TEXTAREA/LABEL/SUMMARY`、
 *    `contenteditable`、`tabindex != -1` 与 `role=button|link|…`，所以按钮照常可点。
 *
 * 2. **绝不自己实现双击最大化。** Tauri 注入的 `drag.js` 已经做了：
 *    `const cmd = e.detail === 2 ? 'internal_toggle_maximize' : 'start_dragging'`。
 *    自己再挂一个双击处理器去调 `toggleMaximize` 会让状态被切换两次，净效果回到原样
 *    ——用户看到的是「双击标题栏没反应」，而代码里明明写了处理。
 *    这是本组件最容易犯且最难看出来的错，因此有一条行为断言直接对着它。
 *
 * 3. **`onPointerDown` 只对非鼠标指针调 `startDragging()`。** 注入脚本只监听鼠标事件，
 *    触摸与手写笔在它视野之外；而对鼠标也调一次就会与它同时发起拖动。
 *    按钮是拖动区的**兄弟节点而不是子节点**，所以笔尖点在按钮上不会冒泡到这里。
 */

import { type WindowChrome } from "./useWindowChrome";
import "./titlebar.css";

export interface TitleBarProps {
  /** 显示在标题栏上的文字。 */
  title: string;
  chrome: WindowChrome;
}

/** 最小化：一条横线。 */
function MinimizeGlyph() {
  return (
    <svg className="titlebar__glyph" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
      <line x1="0.75" y1="5" x2="9.25" y2="5" />
    </svg>
  );
}

/** 最大化 / 还原：一个方框，还原时用两层错开的方框。 */
function MaximizeGlyph({ restore }: { restore: boolean }) {
  if (restore) {
    return (
      <svg className="titlebar__glyph" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
        <rect x="0.75" y="2.75" width="6.5" height="6.5" />
        <path d="M2.75 2.75V0.75H9.25V7.25H7.25" />
      </svg>
    );
  }
  return (
    <svg className="titlebar__glyph" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
      <rect x="0.75" y="0.75" width="8.5" height="8.5" />
    </svg>
  );
}

/** 关闭：一个叉。 */
function CloseGlyph() {
  return (
    <svg className="titlebar__glyph" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
      <line x1="0.75" y1="0.75" x2="9.25" y2="9.25" />
      <line x1="9.25" y1="0.75" x2="0.75" y2="9.25" />
    </svg>
  );
}

export default function TitleBar({ title, chrome }: TitleBarProps) {
  const { platform, isMaximized, showWindowButtons } = chrome;

  return (
    <header className="titlebar" data-platform={platform}>
      {/*
        `deep` 而非裸属性，见文件头第 1 条。
        刻意**没有**双击处理器：见第 2 条。测试会扫这个文件里那个 React 双击属性的
        字面量，所以此处连它的名字都不写出来——todo 59 在 Tauri 官方日志插件那条禁令上
        踩过同一个坑：解释禁令的文字自己命中了禁令。
      */}
      <div
        className="titlebar__drag"
        data-tauri-drag-region="deep"
        data-testid="titlebar-drag-region"
        onPointerDown={(event) => {
          chrome.dragForNonMousePointer(event.pointerType);
        }}>
        <span className="titlebar__title">{title}</span>
      </div>

      {/*
        非 Tauri 宿主（`controls === null`）刻意渲染**零个按钮**而不是三个点了没反应的按钮：
        后者看起来是好的，而这正是最坏的形态。macOS 同理——红绿灯由系统绘制，
        再画一组就是两套控件并存。
      */}
      {showWindowButtons && (
        <div className="titlebar__controls">
          <button
            type="button"
            className="titlebar__button"
            aria-label="最小化"
            title="最小化"
            onClick={chrome.minimize}>
            <MinimizeGlyph />
          </button>
          <button
            type="button"
            className="titlebar__button"
            aria-label={isMaximized ? "向下还原" : "最大化"}
            title={isMaximized ? "向下还原" : "最大化"}
            onClick={chrome.toggleMaximize}>
            <MaximizeGlyph restore={isMaximized} />
          </button>
          <button
            type="button"
            className="titlebar__button titlebar__button--close"
            aria-label="关闭"
            title="关闭"
            onClick={chrome.close}>
            <CloseGlyph />
          </button>
        </div>
      )}
    </header>
  );
}
