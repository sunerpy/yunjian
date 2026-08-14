/**
 * 外壳用的线性图标。
 *
 * 三条约定：
 *
 * 1. **`stroke="currentColor"` + `fill="none"`**，尺寸交给 className。图标因此跟着文字色走，
 *    选中态与悬停态只需要改一处颜色，不必为图标单独维护一组色值。
 * 2. **`aria-hidden` + `focusable="false"`**。每个图标旁边都有真实文字标签（折叠时那段文字
 *    仍在 DOM 里，只是视觉隐藏），所以图标对读屏是纯装饰。少了 `focusable="false"`，
 *    IE/旧 Edge 会把 svg 收进 Tab 序列——Tauri 的 WebView2 不受影响，但 `<svg>` 的这条
 *    属性成本为零，留着比考证宿主便宜。
 * 3. **`stroke-width` 取 1.5**，与自绘标题栏的字形描边同宽（`titlebar.css` 的
 *    `.titlebar__glyph`）。两处不同宽会让侧栏图标与窗口按钮看起来是两套图形。
 */

interface IconProps {
  /** 尺寸与颜色一律由调用方给。刻意不设默认值：一个没写尺寸的图标会按 svg 的
      默认 300×150 撑开，那种缺陷在开发时一眼可见，比默认成某个尺寸后到处不对齐好排查。 */
  className: string;
}

/** 24×24 的公共外壳，避免四个图标各写一遍相同的属性。 */
function Icon({ className, children }: IconProps & { children: React.ReactNode }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false">
      {children}
    </svg>
  );
}

/** 检索：放大镜。 */
export function SearchIcon({ className }: IconProps) {
  return (
    <Icon className={className}>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <line x1="15.5" y1="15.5" x2="20.5" y2="20.5" />
    </Icon>
  );
}

/** 背诵：翻开的书。 */
export function ReciteIcon({ className }: IconProps) {
  return (
    <Icon className={className}>
      <path d="M12 6.5C10 4.8 7.4 4.2 4.5 4.5v13c2.9-.3 5.5.3 7.5 2 2-1.7 4.6-2.3 7.5-2v-13c-2.9-.3-5.5.3-7.5 2Z" />
      <line x1="12" y1="6.5" x2="12" y2="19.5" />
    </Icon>
  );
}

export function DictionaryIcon({ className }: IconProps) {
  return (
    <Icon className={className}>
      <path d="M5 4.5h11.5A2.5 2.5 0 0 1 19 7v12.5H7.5A2.5 2.5 0 0 1 5 17Z" />
      <path d="M5 17a2.5 2.5 0 0 1 2.5-2.5H19" />
      <path d="M9 8h6M12 6v5" />
    </Icon>
  );
}

/**
 * 设置：三条带滑块的横轨。
 *
 * 刻意**不用齿轮**。齿轮的简化式（一圈加八道放射短线）在 18px 下读起来是**太阳或星芒**，
 * 于是它更像「主题切换」而不是「设置」——目视 QA 的第一反应就是这个。
 * 齿要画得像齿就得画梯形齿廓，那在 18px、1.5px 描边下会糊成实心圆。
 * 滑块轨道没有这个问题：三条横线加三个圆点，在小尺寸下仍然是三条横线加三个圆点。
 */
export function SettingsIcon({ className }: IconProps) {
  return (
    <Icon className={className}>
      <line x1="4" y1="7" x2="20" y2="7" />
      <line x1="4" y1="12" x2="20" y2="12" />
      <line x1="4" y1="17" x2="20" y2="17" />
      {/* 三个滑块横向错开，否则三个圆点连成一条竖线，看起来像轨道上的一道划痕。 */}
      <circle cx="9" cy="7" r="2" />
      <circle cx="15" cy="12" r="2" />
      <circle cx="7.5" cy="17" r="2" />
    </Icon>
  );
}

/** 折叠 / 展开：双箭头。`collapsed` 决定朝向。 */
export function CollapseIcon({ className, collapsed }: IconProps & { collapsed: boolean }) {
  return (
    <Icon className={className}>
      {collapsed ? (
        <>
          <path d="M8.5 6.5 14 12l-5.5 5.5" />
          <path d="M14.5 6.5 20 12l-5.5 5.5" />
        </>
      ) : (
        <>
          <path d="M15.5 6.5 10 12l5.5 5.5" />
          <path d="M9.5 6.5 4 12l5.5 5.5" />
        </>
      )}
    </Icon>
  );
}

/** 关闭：一个叉。设置弹窗的关闭按钮用它。 */
export function CloseIcon({ className }: IconProps) {
  return (
    <Icon className={className}>
      <line x1="5.5" y1="5.5" x2="18.5" y2="18.5" />
      <line x1="18.5" y1="5.5" x2="5.5" y2="18.5" />
    </Icon>
  );
}
