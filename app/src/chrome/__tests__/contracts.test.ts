/**
 * 跨文件契约：capability 权限集，与「样式表里不得出现 app-region」。
 *
 * 这两条都不是前端内部的事，却由前端测试持有——理由是知识在这一侧：
 * 「哪个按钮发哪条命令」只有调用方知道，而 capability 文件是被动的授权清单。
 * 放在 Rust 侧只能断言「文件里有这几个字符串」，说不出为什么是这几个。
 */

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// 刻意不用 `import.meta.url`：Vite 会把它改写成一个非 `file:` 的 URL，
// `fileURLToPath` 于是抛 `The URL must be of scheme file`（已实测）。
// vitest 的工作目录是 `app/`（`package.json` 所在处），所以仓库根是它的父目录。
// 下面那条 `Makefile` 探针把「工作目录不对」变成一条可读的失败，而不是一串 ENOENT。
const repoRoot = resolve(process.cwd(), "..");

function read(relative: string): string {
  return readFileSync(resolve(repoRoot, relative), "utf8");
}

describe("测试自身的定位前提", () => {
  it("能从推定的仓库根读到 Makefile", () => {
    expect(existsSync(resolve(repoRoot, "Makefile"))).toBe(true);
  });
});

describe("capabilities 的最小权限集", () => {
  /**
   * 四条按钮/拖动权限，各自对着 `windowControls.ts` 里的一个方法。
   *
   * **`core:window:allow-toggle-maximize` 与 `core:default` 已含的
   * `core:window:allow-internal-toggle-maximize` 是两条不同的命令**：后者是 Tauri 注入的
   * `drag.js` 双击时发的（`plugin:window|internal_toggle_maximize`），前者是最大化按钮发的
   * （`plugin:window|toggle_maximize`，已在 `windowControls.test.ts` 里实测命令名）。
   * 只授后者，按钮点了没反应；只授前者，双击没反应。两种都是 IPC promise 被拒，不报错。
   */
  const REQUIRED_WINDOW_PERMISSIONS = [
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-close",
    "core:window:allow-start-dragging",
  ];

  const capability = JSON.parse(read("crates/yunjian-app/capabilities/default.json")) as {
    permissions: string[];
    windows: string[];
  };

  it("含 core:default 与四条窗口控制权限，一共五项", () => {
    expect(capability.permissions).toContain("core:default");
    for (const permission of REQUIRED_WINDOW_PERMISSIONS) {
      expect(capability.permissions).toContain(permission);
    }
  });

  it("刻意不多授：只有这五项", () => {
    // 权限是攻击面。多授一条不会有任何症状，所以只能靠这条断言把它变成一次可见的决定。
    expect([...capability.permissions].sort()).toEqual(
      ["core:default", ...REQUIRED_WINDOW_PERMISSIONS].sort(),
    );
  });

  it("授权范围是 main 窗口，与两份 tauri 配置里的 label 一致", () => {
    // capabilities 按 label 授权，label 漂移会让权限全部落空且不报错。
    expect(capability.windows).toEqual(["main"]);
  });

  it("「这四条不在默认集里」这条正向对照刻意放在 Rust 侧", () => {
    // `crates/yunjian-app/gen/` 是 tauri-build 的产物且在 `.gitignore` 里（已实测），
    // 一个没跑过 cargo 的检出里 `acl-manifests.json` 不存在，从这里读它只会得到 ENOENT。
    // 那条对照因此住在 `crates/yunjian-app/tests/window_capabilities.rs`，
    // 那里 build.rs 保证产物已生成。这条断言只是把分工钉在被它影响的位置上。
    expect(existsSync(resolve(repoRoot, "crates/yunjian-app/tests/window_capabilities.rs"))).toBe(
      true,
    );
  });
});

describe("app-region 禁令", () => {
  /**
   * `-webkit-app-region` 是 Electron 的 Chromium 分支属性，WebKitGTK 与 WKWebView 完全
   * 没实现；无前缀的 `app-region` 在 Windows WebView2 里确实生效，但它把整块区域都当
   * 标题栏，于是**吞掉区域内每一个按钮的点击**——这正是 Tauri 在 2.0.0-beta.22 撤掉
   * 默认注入的原因。两种形态都不可用，所以扫的是 `app-region` 这个词根。
   */
  const FORBIDDEN = /app-region/;

  function sourceFiles(dir: string): string[] {
    const out: string[] = [];
    for (const entry of readdirSync(resolve(repoRoot, dir), { withFileTypes: true })) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) {
        out.push(...sourceFiles(path));
      } else if (/\.(css|tsx?|html)$/.test(entry.name)) {
        out.push(path);
      }
    }
    return out;
  }

  it("前端源码里没有 app-region（含内联样式与 HTML）", () => {
    const offenders = sourceFiles("app/src")
      // 本文件自己在解释这条禁令，必须排除，否则禁令的执行机制会把自己判成违规
      // ——todo 59 已经在 tauri-plugin-log 那条禁令上踩过同一个坑。
      .filter((path) => !path.endsWith("__tests__/contracts.test.ts"))
      .filter((path) => FORBIDDEN.test(read(path)));

    expect(offenders).toEqual([]);
    expect(FORBIDDEN.test(read("app/index.html"))).toBe(false);
  });

  it("只有测试文件用 Node API：tsconfig 的 node 类型不能漏进 WebView 代码", () => {
    // 本文件与 TitleBar 的源码扫描要读磁盘，所以 `tsconfig.json` 的 `types` 里加了
    // `"node"`。代价是 `process` / `node:fs` 在整个 `src` 下都通过类型检查，
    // 而一处漏进 WebView 代码的 `node:` 导入在 `tsc` 与 `vitest` 里都是绿的，
    // 只有真机上的 WebView 会报模块找不到。这条断言把那个缺口关上。
    const offenders = sourceFiles("app/src")
      .filter((path) => !path.includes("__tests__/"))
      .filter((path) => /from\s+["'`]node:|require\(\s*["'`]node:/.test(read(path)));

    expect(offenders).toEqual([]);
  });

  it("构建出来的样式表里也没有", () => {
    // 源码扫描挡不住经由依赖或 PostCSS 插件注入的样式，所以要看真实产物。
    // `app/dist` 是 cargo 的编译期前置条件（见 Makefile 的 FRONTEND_DIST），
    // 门禁里它必然存在；单独跑 `npm test` 的开发者可能还没构建过，此时跳过并说明。
    const assets = "app/dist/assets";
    // 刻意**不跳过**产物缺失的情况：跳过会让这条断言在没构建过的树上变成空操作，
    // 而门禁恰好是最需要它的地方。`app/dist` 本来就是 cargo 的编译期前置条件
    // （见 Makefile 的 `FRONTEND_DIST`），门禁里必然存在；缺了就该报出来让人去构建。
    expect(
      existsSync(resolve(repoRoot, assets)),
      "app/dist/assets 不存在。先跑 `make frontend`（或 app/ 里 `npm run build`）再验这条。",
    ).toBe(true);
    const stylesheets = readdirSync(resolve(repoRoot, assets)).filter((name) =>
      name.endsWith(".css"),
    );
    expect(stylesheets.length).toBeGreaterThan(0);
    for (const name of stylesheets) {
      expect(FORBIDDEN.test(read(`${assets}/${name}`))).toBe(false);
    }
  });
});

describe("标题栏高度与 macOS 让位量的跨文件耦合", () => {
  it("--titlebar-height 只在令牌层定义一次", () => {
    // 标题栏与其下方内容区共用这一个值。两处各写一个数会得到一条缝或一段重叠，
    // 而这种偏差只在截图上才看得见。
    const declarations = read("app/src/styles.css").match(/--titlebar-height\s*:/g) ?? [];
    expect(declarations).toHaveLength(1);
  });

  it("macOS 的红绿灯让位量与 tauri.macos.conf.json 的 trafficLightPosition 同时存在", () => {
    // 这两个数是一对：红绿灯从 x=12 起、约 70px 宽，让位不足会让标题压在按钮上。
    // 只能钉住「两处都在」，具体像素对不对要靠真机看——但少了任一处一定是错的。
    expect(read("app/src/styles.css")).toContain("--titlebar-macos-inset");
    const overlay = JSON.parse(read("crates/yunjian-app/tauri.macos.conf.json")) as {
      app: { windows: Array<{ trafficLightPosition?: { x: number; y: number } }> };
    };
    expect(overlay.app.windows[0]?.trafficLightPosition).toBeDefined();
  });
});
