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

cargo build --manifest-path "${workspace_dir}/Cargo.toml" \
  -p yunjian-mobile --features uniffi --lib
cargo run --manifest-path "${workspace_dir}/Cargo.toml" \
  -p yunjian-mobile --features uniffi --bin uniffi-bindgen -- \
  generate --library --crate yunjian_mobile --no-format \
  --language kotlin --language swift --out-dir "${output_dir}" \
  "${library_file}"
