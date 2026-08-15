# Installation and release artifacts

The [README](../../README.md) quick start gives three commands. This page has everything it leaves
out: which environment variables the installers honour, how to fetch from a private repository, what
the release pipeline produces, and the per-platform system floor.

## The installer scripts

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.sh | sh
```

```powershell
# Windows
irm https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.ps1 | iex
```

Both scripts detect the OS and CPU architecture, resolve the matching release asset, and **verify its
SHA-256 before anything lands on disk** — a checksum failure installs nothing and exits 3. Both
honour the same variable names; a name that diverges would make the documentation true for `sh` and
false for PowerShell, so a test pins them together
(`crates/yunjian-cli/tests/install_scripts.rs`).

| Variable                    | Default            | Effect                                                          |
| --------------------------- | ------------------ | --------------------------------------------------------------- |
| `YUNJIAN_VERSION`           | latest release     | Install a specific version; `v0.1.0` or `0.1.0`                 |
| `YUNJIAN_INSTALL_DIR`       | `$HOME/.local/bin` | Install directory                                               |
| `YUNJIAN_BASE_URL`          | GitHub Release     | Download base, for internal mirrors and offline tests           |
| `YUNJIAN_API_URL`           | GitHub API         | API base used for version resolution                            |
| `GH_TOKEN` / `GITHUB_TOKEN` | none               | Download a private Release through `gh`; or run `gh auth login` |

## Private repositories

Install the GitHub CLI first, then provide a token that can read the repository's Releases, or
authenticate with `gh auth login`:

```bash
GH_TOKEN=github_pat_xxx sh scripts/install.sh
```

```powershell
$env:GH_TOKEN = 'github_pat_xxx'
.\scripts\install.ps1
```

The token is handed only to the GitHub CLI and is never written to the installer's temporary
directory. Custom `YUNJIAN_BASE_URL` / `YUNJIAN_API_URL` endpoints continue to use ordinary HTTP
downloads.

## CLI archives

The release pipeline produces these CLI archives with `voice,mcp` explicitly enabled. Linux artifacts
must be musl executables with no dynamic dependencies; macOS and Windows archives also carry the native
voice libraries they need:

| OS      | Target                       | Archive  |
| ------- | ---------------------------- | -------- |
| Linux   | `x86_64-unknown-linux-musl`  | `tar.gz` |
| Linux   | `aarch64-unknown-linux-musl` | `tar.gz` |
| macOS   | `x86_64-apple-darwin`        | `tar.gz` |
| macOS   | `aarch64-apple-darwin`       | `tar.gz` |
| Windows | `x86_64-pc-windows-msvc`     | `zip`    |
| Windows | `aarch64-pc-windows-msvc`    | `zip`    |

Both Linux targets are built with `cargo-zigbuild`, and the release gate uses `readelf` to reject an
executable with any `NEEDED` dynamic dependency. The installer prefers musl assets and retains GNU
asset candidates only for compatibility with older releases.

## Desktop installers and automatic updates

Desktop assets are `.deb` and `.AppImage` for Linux x86_64, `.dmg` and updater `.app.tar.gz` for
Apple Silicon macOS, and NSIS `.exe` plus `.msi` for Windows x86_64. Tauri updates declare only
`linux-x86_64`, `darwin-aarch64`, and `windows-x86_64-nsis`. Every installer, signature,
`latest.json`, and CLI archive has a matching `.sha256` file.

Per-platform system floors, the microphone permission chain, and behaviour below the floor are in
[Platform requirements](PLATFORM-REQUIREMENTS.md); the licensing consequences of enabling `voice` are
in [Voice build](../VOICE-BUILD.zh.md).

## Before the first release exists

The first tagged release (`v0.1.0`) has not been cut yet, so the commands above are waiting on it.
Until then, build from source:

```bash
cargo build --workspace --release -p yunjian-cli
# the binary lands at target/release/yunjian
```

The developer workflow is in [Development](DEVELOPMENT.md).
