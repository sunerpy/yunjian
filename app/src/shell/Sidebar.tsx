/**
 * 左侧竖向导航。取代了原先横在标题栏下方的那一条。
 *
 * # 为什么从横排改成侧栏
 *
 * 横排导航条占掉整宽 × 约 56px 的一整条，而它只装三个入口；在一个纵向内容（诗词正文、
 * 复习队列、设置面板）为主的窗口里，那一条是纯损耗。侧栏把同样的入口放进 3.5rem 的
 * 竖条里，且折叠后仍然可用——横排导航没有「折叠」这个形态。
 *
 * # 三个 testid 与 `aria-current` 为什么必须原样保留
 *
 * `__tests__/App.test.tsx` 的 11 条断言全部只用用户看得见的东西导航（点按钮），
 * 那一组存在的理由是一次真实缺口：设置界面 14 个文件全部实现、37 条断言全绿，
 * **但页面上没有任何入口能到它**。所以 `app-nav` / `nav-search` / `nav-recite` /
 * `nav-settings` 这四个 testid 是那组断言的抓手，换掉它们等于把那道防线拆了。
 *
 * # 选中态用 `aria-current`，设置用 `aria-expanded`
 *
 * 检索与背诵是两屏，选中态是「当前在哪一屏」，`aria-current="page"` 正是这个意思。
 * 设置改成了弹窗，它**不是一屏**——弹窗打开时底下那一屏仍然是当前页。给它写
 * `aria-current="page"` 会同时出现两个「当前页」，读屏用户拿到的是矛盾信息。
 * 所以设置那个按钮用 `aria-haspopup="dialog"` + `aria-expanded`，这也是它的真实语义。
 *
 * # 折叠时文字留在 DOM 里
 *
 * 折叠不是「删掉标签」而是「只在视觉上收起」：标签用 `sr-only` 移出视觉流，
 * 读屏仍然读得到，无障碍名与 `textContent` 在两种形态下完全一致。
 * 用 `{collapsed ? null : <span>…</span>}` 会让折叠态下的按钮变成一个无名图标钮。
 */

import { CollapseIcon, ReciteIcon, SearchIcon, SettingsIcon } from "./icons";

/** 侧栏能直接抵达的两屏。阅读页归「检索」那一支：它是从检索进去的。 */
export type ShellSection = "search" | "recite";

export interface SidebarProps {
  section: ShellSection;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onSelect: (section: ShellSection) => void;
  /** 设置弹窗是否打开。只用于 `aria-expanded`，侧栏自己不持这个状态。 */
  settingsOpen: boolean;
  onOpenSettings: () => void;
}

/**
 * 一行导航项的公共样式。选中与未选中只差颜色与左侧标尺，几何量刻意相同——
 * 几何量也变的话，切换选中项会让整条侧栏的行位轻微跳动。
 *
 * `focus-visible` 那一段是必需的：不写的话键盘焦点落在这里时是浏览器默认的蓝色描边，
 * 那个蓝与本项目的 `--color-accent` 不是一个色系，在一屏暖灰里非常突兀
 * （目视 QA 抓到的）。`outline-offset` 取负值把描边收进按钮内侧，
 * 否则它会盖住左侧那条 2px 的选中标尺。
 */
const ITEM_BASE =
  "flex w-full cursor-pointer items-center gap-3 border-l-2 py-2 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--color-accent)]";

/**
 * 行内的横向布局：展开态左对齐，折叠态居中。折叠开关也用它，于是四个图标在两种形态下
 * 始终共享同一列。
 *
 * 折叠态**不能沿用展开态那个左内边距**：那会让图标停在距左 16px（中心 25px），
 * 而 56px 宽的条中心是 28px——目视 QA 量到的「图标偏左 3px」就是这么来的。
 */
function layoutClass(collapsed: boolean): string {
  return collapsed ? "justify-center" : "pl-[calc(var(--space-4)-2px)] pr-3 text-left";
}

/**
 * 选中项的底色是 `--color-surface`，而侧栏自己的底色是 `--color-bg`——**两者不能反过来**。
 *
 * 第一版把侧栏做成 `--color-surface`、选中项做成 `--color-surface-raised`，
 * 在深色下勉强能看（Δ≈7/通道），**在浅色下这两个值是 #ffffff 与 #fffefb，Δ≈4**，
 * 于是选中态的底色信号等于不存在，全靠那条 2px 标尺撑着。这个缺陷只在浅色配色下出现，
 * 而深色是本机的默认配色——所以它是切到浅色目视时才暴露的。
 *
 * 现在这一对在两套配色下都有可见差距（浅色 #f7f5f1 对 #ffffff，深色 #14171a 对 #1b1f23），
 * 加上左侧标尺与字重，选中态一共三个维度。`__tests__/tailwind.test.ts` 有一条断言
 * 钉住这两个令牌在浅色与深色里都取不同的值。
 */
function itemClass(active: boolean, collapsed: boolean): string {
  const layout = `${ITEM_BASE} ${layoutClass(collapsed)}`;
  return active
    ? `${layout} border-[var(--color-accent)] bg-[var(--color-surface)] font-medium text-[var(--color-text)]`
    : `${layout} border-transparent text-[var(--color-text-muted)] hover:bg-[var(--color-titlebar-hover)] hover:text-[var(--color-text)]`;
}

export default function Sidebar({
  section,
  collapsed,
  onToggleCollapsed,
  onSelect,
  settingsOpen,
  onOpenSettings,
}: SidebarProps) {
  const iconClass = "size-[1.125rem] shrink-0";
  const labelClass = collapsed ? "sr-only" : "truncate";

  return (
    <nav
      aria-label="主导航"
      data-testid="app-nav"
      data-collapsed={collapsed ? "true" : "false"}
      // 本项目刻意没有引入 Tailwind 的 preflight，所以按钮会保留 UA 的灰底与 2px 立体边框。
      // 这个属性把 `tailwind.css` 里那条作用域复位打开。理由与实测症状见那份文件顶部。
      data-shell-chrome=""
      className={`flex shrink-0 flex-col gap-1 border-r border-[var(--color-border)] bg-[var(--color-bg)] py-2 transition-[width] duration-150 ${
        collapsed ? "w-14" : "w-52"
      }`}>
      <button
        type="button"
        data-testid="sidebar-toggle"
        aria-expanded={!collapsed}
        aria-controls="app-nav-items"
        aria-label={collapsed ? "展开侧栏" : "折叠侧栏"}
        title={collapsed ? "展开侧栏" : "折叠侧栏"}
        onClick={onToggleCollapsed}
        // 与导航项共用 `layoutClass`：不这么做的话，居中的开关在展开态落在 x≈104、
        // 折叠态落在 x≈28，同一个控件横移 76px，看起来像换了位置的另一个控件
        // （目视 QA 量到的）。左边那条透明的 2px 边框是为了与导航项的选中标尺占位对齐。
        className={`mb-1 flex w-full cursor-pointer items-center gap-3 border-l-2 border-transparent py-2 text-[var(--color-text-muted)] transition-colors hover:bg-[var(--color-titlebar-hover)] hover:text-[var(--color-text)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--color-accent)] ${layoutClass(collapsed)}`}>
        <CollapseIcon className={iconClass} collapsed={collapsed} />
      </button>

      {/* `list-none` 要显式写：本项目刻意没有引入 Tailwind 的 preflight，
          所以 `<ul>` 的默认项目符号与缩进都还在。理由见 `tailwind.css`。 */}
      <ul id="app-nav-items" className="m-0 flex list-none flex-col gap-1 p-0">
        <li>
          <button
            type="button"
            data-testid="nav-search"
            // 详情页也归这一支：把它算作第三个并列项会让导航出现一个用户无法直接抵达的条目。
            aria-current={section === "search" ? "page" : undefined}
            title={collapsed ? "检索" : undefined}
            onClick={() => {
              onSelect("search");
            }}
            className={itemClass(section === "search", collapsed)}>
            <SearchIcon className={iconClass} />
            <span className={labelClass}>检索</span>
          </button>
        </li>
        <li>
          <button
            type="button"
            data-testid="nav-recite"
            aria-current={section === "recite" ? "page" : undefined}
            title={collapsed ? "背诵" : undefined}
            onClick={() => {
              onSelect("recite");
            }}
            className={itemClass(section === "recite", collapsed)}>
            <ReciteIcon className={iconClass} />
            <span className={labelClass}>背诵</span>
          </button>
        </li>
      </ul>

      {/*
        设置沉到底部：它是一次性配置而不是日常在用的一屏，与上面两个并排会让三者看起来
        是同一类东西。`mt-auto` 把它推到底，同时上方一条细线把它与导航项分开。
      */}
      <div className="mt-auto border-t border-[var(--color-border)] pt-2">
        <button
          type="button"
          data-testid="nav-settings"
          aria-haspopup="dialog"
          aria-expanded={settingsOpen}
          title={collapsed ? "设置" : undefined}
          onClick={onOpenSettings}
          className={itemClass(settingsOpen, collapsed)}>
          <SettingsIcon className={iconClass} />
          <span className={labelClass}>设置</span>
        </button>
      </div>
    </nav>
  );
}
