// `defineConfig` 取自 `vitest/config` 而非 `vite`：只有前者的类型里有 `test` 段。
// 用 `vite` 那一个会得到 TS2769「'test' does not exist in type 'UserConfigExport'」，
// 而这条错误在 `vite build` 里不出现——它只在 `tsc --noEmit` 里出现，
// 所以 `npm run build` 刻意把类型检查放在构建之前。
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri 期望前端在固定端口上，且不允许它自动改端口 —— `tauri.conf.json` 的
// `devUrl` 是写死的 http://localhost:5173，端口漂移会让 `cargo tauri dev` 连到空地址。
// 因此 `strictPort: true`：端口被占时报错，而不是静默换一个。
export default defineConfig({
  plugins: [react()],
  // Tauri 自己会把 stderr 上的 Rust 日志交给终端。Vite 的 clearScreen 会把它们擦掉，
  // 于是「窗口起不来」的原因刚打出来就消失。
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    // WebView2（Windows 10 1809 起）与 WebKitGTK 2.38+ 都支持 ES2022。
    // 刻意不用 esnext：那会把可用性交给宿主 WebView 的版本，而平台底线是已定案的
    // （docs/PLATFORM-REQUIREMENTS.zh.md），构建目标必须与它一致而不是更激进。
    target: "es2022",
    // 生产构建保留 sourcemap：真机验收（todo 67）里一条 WebView 报错如果没有 sourcemap，
    // 就只能拿到压缩后的行号，等于没有诊断。
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
  },
});
