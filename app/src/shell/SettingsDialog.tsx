/**
 * 设置弹窗。设置原先是外壳的第三屏，现在收进一个模态对话框。
 *
 * # 为什么改成弹窗
 *
 * 设置是一次性配置，而检索与背诵是日常在用的两屏。把它做成第三屏意味着「去改个模型名」
 * 要先离开当前在读的诗，回来时那一屏的滚动位置与检索条件都没了。弹窗打开时底下那一屏
 * **仍然挂载着**，关掉就回到原处——这条正是 `App.test.tsx` 里那条新断言盯的东西。
 *
 * # 用原生 `<dialog>`，但每一条行为都自己实现
 *
 * 真实 WebView 里 `showModal()` 给的是顶层（top layer）与 `::backdrop`，那两样自己搭不出来
 * （`z-index` 打不过自绘标题栏这类已有层叠上下文的所有情形）。所以壳子用原生元素。
 *
 * 但**行为不能交给原生**：
 *
 * - **jsdom 30 没有实现 `HTMLDialogElement` 的方法。** `showModal` / `show` / `close`
 *   三个全是 `undefined`（本机实测；`open` 属性的反射是好的）。若 Esc、焦点陷阱、
 *   遮罩关闭都靠原生，这四条行为在 313 条前端测试里**一条都验不到**，
 *   而它们恰恰是这个弹窗能不能用的全部。
 * - 原生只给 Esc 与遮罩，**不给焦点陷阱之外的焦点归还**，也不给「关闭后焦点回到触发按钮」。
 *
 * 因此：壳子用原生元素，`open` 用 `showModal()`（拿不到就退回写 `open` 属性，
 * 那条分支只在 jsdom 里走到），Esc / 焦点陷阱 / 遮罩点击 / 焦点归还四条全部显式实现。
 *
 * # 为什么关闭时不渲染内容
 *
 * 四块面板的 effect 一挂载就发查询（语料库、语音模型、缓存统计），真实路径上要开 SQLite。
 * 若弹窗关着也把它们挂上，那三条查询会在应用启动时就跑——用户还没打开设置。
 * 所以 `<dialog>` 常驻 DOM（避免开合时的挂载竞态），内容只在 `open` 时渲染。
 */

import { useCallback, useEffect, useRef } from "react";
import type { SettingsPorts } from "../data/settingsPorts";
import SettingsScreen from "../settings/SettingsScreen";
import { CloseIcon } from "./icons";

export interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
  ports: SettingsPorts;
  templateVersion?: string;
}

/**
 * 可获得焦点的元素。
 *
 * `[tabindex]:not([tabindex="-1"])` 那一段是必需的：设置面板里的 `<dl>` 与说明段落不可聚焦，
 * 但将来若有人加了一个 `tabindex="0"` 的自定义控件，漏掉它会让焦点陷阱把那个控件跳过去。
 */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusableWithin(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)];
}

export default function SettingsDialog({
  open,
  onClose,
  ports,
  templateVersion,
}: SettingsDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  /** 打开前的焦点落点，关闭时归还给它。 */
  const restoreTo = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const element = dialogRef.current;
    if (!element) {
      return;
    }

    if (open) {
      restoreTo.current =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
      if (typeof element.showModal === "function") {
        element.showModal();
      } else {
        // jsdom 分支：见文件头。`open` 属性让内容可查询，行为由下面几个处理器承担。
        element.setAttribute("open", "");
      }
      // 打开后焦点落到弹窗内第一个可聚焦元素（关闭按钮），否则焦点还留在侧栏那个按钮上，
      // 于是 Tab 会从弹窗外面开始走，焦点陷阱第一圈就是空的。
      focusableWithin(element)[0]?.focus();
      return;
    }

    if (element.hasAttribute("open")) {
      if (typeof element.close === "function") {
        element.close();
      } else {
        element.removeAttribute("open");
      }
      restoreTo.current?.focus();
      restoreTo.current = null;
    }
  }, [open]);

  /**
   * 焦点陷阱：Tab 在弹窗内循环。
   *
   * 真实浏览器的 `showModal()` 已经把焦点限制在弹窗内，这里是重复的——但它是**唯一能被
   * jsdom 验证的那一份**，而重复的代价只是几行不会互相冲突的边界判断
   * （原生把焦点困在弹窗里，这里只决定困住之后的下一站是谁）。
   */
  const trapTab = useCallback((event: React.KeyboardEvent<HTMLDialogElement>) => {
    if (event.key !== "Tab") {
      return;
    }
    const element = dialogRef.current;
    if (!element) {
      return;
    }
    const items = focusableWithin(element);
    if (items.length === 0) {
      return;
    }
    const first = items[0];
    const last = items.at(-1);
    if (!first || !last) {
      return;
    }
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !element.contains(active))) {
      event.preventDefault();
      last.focus();
      return;
    }
    if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }, []);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDialogElement>) => {
      if (event.key === "Escape") {
        // `preventDefault` 是为了让关闭**只有一条路径**：浏览器的原生 Esc 也会关 dialog，
        // 那会让元素先自己关掉、React 的 `open` 再变 false，两条路径的顺序不可控。
        event.preventDefault();
        onClose();
        return;
      }
      trapTab(event);
    },
    [onClose, trapTab],
  );

  return (
    <dialog
      ref={dialogRef}
      data-testid="settings-dialog"
      aria-label="设置"
      // 见 `Sidebar.tsx` 上同名属性的说明：打开 `tailwind.css` 里那条作用域复位。
      data-shell-chrome=""
      onKeyDown={handleKeyDown}
      onCancel={(event) => {
        // 原生 Esc 走 `cancel`。同样交给 React 关，理由见 `handleKeyDown`。
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        // 点在遮罩上时事件目标就是 `<dialog>` 自己（内容在下面那层 `div` 里）。
        // 点在内容上时目标是那个 div 或更深的元素，于是这条不成立。
        if (event.target === dialogRef.current) {
          onClose();
        }
      }}
      /* 三处取值都是目视 QA 之后改的，不是一次写对的：
         - 底色用 `--color-surface` 而不是 `--color-bg`。后者与页面底色相同，
           于是弹窗与被压暗的页面在深色配色下几乎同调，看不出层级。
         - 遮罩 60% 而不是 45%：45% 压不住本来就很深的深色页面。
         - 一圈外阴影。没有阴影令牌，所以这里直接给值；深色下边框与页面底色的对比很弱，
           阴影是弹窗与页面之间唯一还剩下的深度信号。 */
      className="m-auto max-h-[85vh] w-[min(46rem,calc(100vw-var(--space-8)))] overflow-hidden rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-surface)] p-0 text-[var(--color-text)] shadow-[0_1.5rem_3rem_rgba(0,0,0,0.45)] backdrop:bg-[color-mix(in_srgb,black_60%,transparent)]">
      {open && (
        <div className="grid max-h-[85vh] grid-rows-[auto_1fr]">
          <div className="flex items-center justify-between gap-4 border-b border-[var(--color-border)] bg-[var(--color-surface-raised)] px-6 py-3">
            {/* 标题放在头部条上，同时让 `SettingsScreen` 关掉它自己那个 `h1`。
                两处都渲染会得到一个空头部条紧跟一个重复标题，中间白空约 70px。 */}
            <h2 className="m-0 font-sans text-lg font-semibold">设置</h2>
            <button
              type="button"
              data-testid="settings-dialog-close"
              aria-label="关闭设置"
              onClick={onClose}
              className="flex cursor-pointer items-center rounded-[var(--radius-sm)] p-1 text-[var(--color-text-muted)] transition-colors hover:bg-[var(--color-titlebar-hover)] hover:text-[var(--color-text)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--color-accent)]">
              <CloseIcon className="size-[1.125rem]" />
            </button>
          </div>
          <div className="min-h-0 overflow-y-auto bg-[var(--color-bg)]">
            <SettingsScreen
              ports={ports}
              showTitle={false}
              {...(templateVersion ? { templateVersion } : {})}
            />
          </div>
        </div>
      )}
    </dialog>
  );
}
