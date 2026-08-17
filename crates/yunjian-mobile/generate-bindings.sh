#!/usr/bin/env bash
set -euo pipefail

crate_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workspace_dir="$(cd -- "${crate_dir}/../.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-${workspace_dir}/target}"
output_dir="${crate_dir}/bindings/generated"

case "$(uname -s)" in
  Darwin) library_file="${target_dir}/debug/libyunjian_mobile.dylib" ;;
  Linux) library_file="${target_dir}/debug/libyunjian_mobile.so" ;;
  *)
    printf '%s\n' "生成 UniFFI 绑定需要 Linux 或 macOS 宿主" >&2
    exit 1
    ;;
esac

rm -rf -- "${output_dir}"
mkdir -p -- "${output_dir}"

# 用 `native-voice` 而不是 `uniffi` 生成。
#
# 生成物必须与**实际分发的那个 `.so`** 的导出面一致，而移动端默认开语音
# （todo 69：`Voice ships on mobile in both branches`）。用 `uniffi` 生成会少掉
# `fetch_voice_model` 这类只在语音构建里存在的入口，于是 Kotlin 侧编译通过、运行时
# 却调不到——那种错误在生成阶段看不出来。
features="${YUNJIAN_BINDINGS_FEATURES:-native-voice}"

cargo build --manifest-path "${workspace_dir}/Cargo.toml" \
  -p yunjian-mobile --features "${features}" --lib
cargo run --manifest-path "${workspace_dir}/Cargo.toml" \
  -p yunjian-mobile --features "${features}" --bin uniffi-bindgen -- \
  generate --library --crate yunjian_mobile --no-format \
  --language kotlin --language swift --out-dir "${output_dir}" \
  "${library_file}"
