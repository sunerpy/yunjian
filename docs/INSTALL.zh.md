# 安装与发布产物

根 [README](../README.md) 的「快速开始」只给三条命令。这里是它省掉的全部细节：安装脚本认哪些
环境变量、私有仓库怎么取、发布管线出哪些产物、以及每个平台的系统底线。

## 安装脚本

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.sh | sh
```

```powershell
# Windows
irm https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.ps1 | iex
```

两个脚本都检测系统与 CPU 架构、挑出对应的发布产物、**校验 SHA-256 之后才落盘**，校验不过就
一个文件也不装（退出 3）。两者认同一组环境变量名——名字分叉会让「文档照 sh 写、Windows
用户照做无效」，因此有一条测试钉住它们一致（`crates/yunjian-cli/tests/install_scripts.rs`）。

| 变量                        | 缺省               | 作用                                                         |
| --------------------------- | ------------------ | ------------------------------------------------------------ |
| `YUNJIAN_VERSION`           | 最新正式发布       | 装指定版本，`v0.1.0` 与 `0.1.0` 都收                         |
| `YUNJIAN_INSTALL_DIR`       | `$HOME/.local/bin` | 安装目录                                                     |
| `YUNJIAN_BASE_URL`          | GitHub Release     | 下载基址，便于内部镜像与离线测试                             |
| `YUNJIAN_API_URL`           | GitHub API         | 版本解析用的 API 基址                                        |
| `GH_TOKEN` / `GITHUB_TOKEN` | 无                 | 通过 GitHub CLI 下载私有仓库 Release；也可先 `gh auth login` |

## 私有仓库

必须先安装 `gh`，再提供一个能读取该仓库 Release 的 token，或提前完成 `gh auth login`：

```bash
GH_TOKEN=github_pat_xxx sh scripts/install.sh
```

```powershell
$env:GH_TOKEN = 'github_pat_xxx'
.\scripts\install.ps1
```

token 只交给 GitHub CLI，不写入安装脚本的临时目录。自定义 `YUNJIAN_BASE_URL` /
`YUNJIAN_API_URL` 时仍使用普通 HTTP 下载。

## CLI 归档

发布管线生成以下 CLI 归档，全部启用 `voice,mcp`，并把 voice 所需动态库放在归档内：

| 系统    | 目标                        | 归档     |
| ------- | --------------------------- | -------- |
| Linux   | `x86_64-unknown-linux-gnu`  | `tar.gz` |
| Linux   | `aarch64-unknown-linux-gnu` | `tar.gz` |
| macOS   | `x86_64-apple-darwin`       | `tar.gz` |
| macOS   | `aarch64-apple-darwin`      | `tar.gz` |
| Windows | `x86_64-pc-windows-msvc`    | `zip`    |

Linux CLI 以 glibc 2.31 为上限。sherpa-onnx 没有可用于 `voice` 的 musl 预编译库，所以安装脚本
会先兼容探测旧版 musl 资产，再回退到当前 GNU 资产——回退链本身有测试盯着。

## 桌面安装包与自动更新

桌面端另发布 Linux x86_64 的 `.deb` 与 `.AppImage`、macOS Apple Silicon 的 `.dmg` 与 updater
`.app.tar.gz`、Windows x86_64 的 NSIS `.exe` 与 `.msi`。Tauri 自动更新只声明 `linux-x86_64`、
`darwin-aarch64` 和 `windows-x86_64-nsis`；每个安装包、签名、`latest.json` 与 CLI 归档都有
同名 `.sha256`。

各平台的系统最低版本、麦克风授权链与低于底线时的降级行为见
[平台要求](PLATFORM-REQUIREMENTS.zh.md)；`voice` 开启后的许可影响见
[语音构建](VOICE-BUILD.zh.md)。

## 还没有正式发布时

首个正式发布（`v0.1.0`）还没切出来，上面的安装命令要等它。在此之前从源码构建：

```bash
cargo build --workspace --release -p yunjian-cli
# 可执行文件在 target/release/yunjian
```

开发者流程见 [开发流程](DEVELOPMENT.zh.md)。
