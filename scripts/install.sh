#!/bin/sh
# 云笺 yunjian 安装脚本（POSIX sh）。
#
# 一条命令装好 CLI，然后把用户交给 `yunjian corpus fetch`。
#
#   curl -fsSL https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.sh | sh
#
# 可用环境变量：
#
#   YUNJIAN_VERSION       要装的版本，形如 `v0.1.0` 或 `0.1.0`。缺省取最新正式发布。
#   YUNJIAN_INSTALL_DIR   安装目录。缺省 `$HOME/.local/bin`。
#   YUNJIAN_BASE_URL      发布资产的下载前缀。缺省 GitHub Releases；改它是为了能对着
#                         本地 mock 服务器验证这个脚本本身。
#   YUNJIAN_API_URL       解析最新版本用的 API 前缀。缺省 GitHub API。
#
# 退出码与 `yunjian` 自身的约定一致（见 docs/CLI.zh.md）：
#
#   0  装好了
#   2  用法错误：平台不受支持、版本号写错、缺少下载或摘要工具
#   3  取不到东西：下载失败、资产不存在、**校验和不匹配**
#
# 校验和不匹配走 3 而不是 2 是刻意的：调用方改命令没用，要改的是「拿到的那份文件」。
# 且任何一次校验失败都**不落盘**——先在临时目录里校验，通过了才装进目标目录。

set -eu

REPO="sunerpy/yunjian"
BINARY="yunjian"

BASE_URL="${YUNJIAN_BASE_URL:-https://github.com/${REPO}/releases/download}"
API_URL="${YUNJIAN_API_URL:-https://api.github.com/repos/${REPO}}"
INSTALL_DIR="${YUNJIAN_INSTALL_DIR:-${HOME}/.local/bin}"

# 日志一律走 stderr，stdout 留给可能的管道消费方。与 CLI 的两条流约定同源。
info() { printf '%s\n' "$*" >&2; }
die_usage() {
  printf 'error: %s\n' "$*" >&2
  exit 2
}
die_unavailable() {
  printf 'error: %s\n' "$*" >&2
  exit 3
}

# ---------------------------------------------------------------- 平台检测

detect_target_candidates() {
  detect_os="$(uname -s)"
  detect_arch="$(uname -m)"

  case "${detect_arch}" in
    x86_64 | amd64) detect_arch="x86_64" ;;
    aarch64 | arm64) detect_arch="aarch64" ;;
    *) die_usage "不支持的 CPU 架构 ${detect_arch}；发布产物只覆盖 x86_64 与 aarch64" ;;
  esac

  case "${detect_os}" in
    # musl 优先：静态链接，不挑 glibc 版本。gnu 是兼容旧发布的回退。
    Linux)
      printf '%s-unknown-linux-musl %s-unknown-linux-gnu\n' \
        "${detect_arch}" "${detect_arch}"
      ;;
    Darwin) printf '%s-apple-darwin\n' "${detect_arch}" ;;
    *)
      die_usage "不支持的系统 ${detect_os}；Windows 请用 scripts/install.ps1"
      ;;
  esac
}

# ---------------------------------------------------------------- 下载与摘要

# 选一个下载工具。curl 与 wget 二者有其一即可。
detect_downloader() {
  if command -v curl >/dev/null 2>&1; then
    printf 'curl\n'
  elif command -v wget >/dev/null 2>&1; then
    printf 'wget\n'
  else
    die_usage "需要 curl 或 wget 之一来下载发布产物"
  fi
}

# 下载 $1 到 $2。失败即返回非 0，由调用方决定是致命还是可回退。
fetch() {
  fetch_url="$1"
  fetch_out="$2"
  case "${DOWNLOADER}" in
    curl) curl -fsSL -o "${fetch_out}" "${fetch_url}" ;;
    wget) wget -q -O "${fetch_out}" "${fetch_url}" ;;
    *) die_usage "未知下载工具 ${DOWNLOADER}" ;;
  esac
}

# 算 $1 的 SHA-256，只输出十六进制摘要。
sha256_of() {
  sha_file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${sha_file}" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${sha_file}" | cut -d' ' -f1
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "${sha_file}" | sed 's/.*= *//'
  else
    die_usage "需要 sha256sum、shasum 或 openssl 之一来校验产物摘要"
  fi
}

# ---------------------------------------------------------------- 版本解析

resolve_version() {
  if [ -n "${YUNJIAN_VERSION:-}" ]; then
    # 两种写法都收：`v0.1.0` 与 `0.1.0`。
    case "${YUNJIAN_VERSION}" in
      v*) printf '%s\n' "${YUNJIAN_VERSION}" ;;
      *) printf 'v%s\n' "${YUNJIAN_VERSION}" ;;
    esac
    return 0
  fi

  resolve_body="$(mktemp)"
  if ! fetch "${API_URL}/releases/latest" "${resolve_body}"; then
    rm -f "${resolve_body}"
    die_unavailable "取不到最新版本；用 YUNJIAN_VERSION 显式指定，例如 YUNJIAN_VERSION=v0.1.0"
  fi
  # 不引 jq：安装脚本的依赖越少越好。`tag_name` 是一个扁平字符串字段，
  # sed 足够，且取不到时下面立刻报错而不是装出一个空版本。
  resolve_tag="$(
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      "${resolve_body}" | head -n 1
  )"
  rm -f "${resolve_body}"
  if [ -z "${resolve_tag}" ]; then
    die_unavailable "最新发布里读不到 tag_name；用 YUNJIAN_VERSION 显式指定版本"
  fi
  printf '%s\n' "${resolve_tag}"
}

# ---------------------------------------------------------------- 主流程

DOWNLOADER="$(detect_downloader)"
TAG="$(resolve_version)"
VERSION="${TAG#v}"
CANDIDATES="$(detect_target_candidates)"

WORK_DIR="$(mktemp -d)"
cleanup() { rm -rf "${WORK_DIR}"; }
trap cleanup EXIT INT TERM

info "云笺 yunjian ${TAG}"

# 逐个候选目标试下载。musl 拿不到就回退 gnu；全都拿不到才算失败。
ARCHIVE=""
TARGET=""
for candidate in ${CANDIDATES}; do
  probe="${BINARY}-${VERSION}-${candidate}.tar.gz"
  info "尝试 ${probe}"
  if fetch "${BASE_URL}/${TAG}/${probe}" "${WORK_DIR}/${probe}"; then
    ARCHIVE="${probe}"
    TARGET="${candidate}"
    break
  fi
  rm -f "${WORK_DIR}/${probe}"
done

if [ -z "${ARCHIVE}" ]; then
  die_unavailable "在 ${TAG} 下找不到适配本机的产物（试过：${CANDIDATES}）"
fi

# 摘要文件是**必需**的，取不到就中止。没有摘要的安装等于没有校验，
# 而「悄悄跳过校验」比「装不上」危险得多。
if ! fetch "${BASE_URL}/${TAG}/${ARCHIVE}.sha256" "${WORK_DIR}/${ARCHIVE}.sha256"; then
  die_unavailable "取不到 ${ARCHIVE}.sha256；缺少摘要时拒绝安装"
fi

EXPECTED="$(cut -d' ' -f1 <"${WORK_DIR}/${ARCHIVE}.sha256")"
ACTUAL="$(sha256_of "${WORK_DIR}/${ARCHIVE}")"
if [ -z "${EXPECTED}" ]; then
  die_unavailable "${ARCHIVE}.sha256 里读不出摘要"
fi
if [ "${EXPECTED}" != "${ACTUAL}" ]; then
  printf 'error: %s 校验和不匹配，未安装任何文件\n' "${ARCHIVE}" >&2
  printf '  期望 %s\n' "${EXPECTED}" >&2
  printf '  实际 %s\n' "${ACTUAL}" >&2
  exit 3
fi
info "校验和通过（sha256 ${ACTUAL}）"

tar -xzf "${WORK_DIR}/${ARCHIVE}" -C "${WORK_DIR}"
if [ ! -f "${WORK_DIR}/${BINARY}" ]; then
  die_unavailable "${ARCHIVE} 里没有 ${BINARY} 可执行文件"
fi

mkdir -p "${INSTALL_DIR}"
chmod +x "${WORK_DIR}/${BINARY}"
# 先搬到同目录的临时名再 mv：正在运行的旧进程不会看到一个半截的文件。
mv "${WORK_DIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}.new"
mv "${INSTALL_DIR}/${BINARY}.new" "${INSTALL_DIR}/${BINARY}"

info "已安装 ${INSTALL_DIR}/${BINARY}（${TARGET}）"

# ---------------------------------------------------------------- 下一步

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    info ""
    info "注意：${INSTALL_DIR} 不在 PATH 上。把这行加进 shell 配置："
    info "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac

info ""
info "下一步："
info "  yunjian corpus fetch      # 下载并校验语料库（约 211 MiB）"
info "  yunjian search 明月       # 查一句试试"
