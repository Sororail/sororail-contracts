#!/usr/bin/env bash
set -euo pipefail

directory="${1:-target/wasm32v1-none/release}"
max_bytes="${WASM_MAX_BYTES:-${MAX_WASM_BYTES:-262144}}"
shopt -s nullglob
files=("$directory"/sororail_*.wasm)
if ((${#files[@]} == 0)); then
  echo "No contract WASM files found in $directory" >&2
  exit 1
fi

failed=0
for file in "${files[@]}"; do
  size=$(stat -c%s "$file")
  name=$(basename "$file")
  echo "$name: $size bytes (limit: $max_bytes)"
  if ((size > max_bytes)); then
    echo "::error file=$file::${name} is ${size} bytes, exceeds ${max_bytes} byte limit" >&2
    failed=1
  fi
done
exit "$failed"
