#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
deployment_file="${1:-$root/DEPLOYMENTS.md}"
fail=0

test -f "$deployment_file" || {
  echo "DEPLOYMENTS.md is missing: $deployment_file" >&2
  exit 1
}

for contract_dir in "$root"/contracts/*/; do
  contract_file="${contract_dir}src/contract.rs"
  [ -f "$contract_file" ] || continue

  contract="${contract_dir%/}"
  contract="${contract##*/}"
  if ! grep -Fq "| \`$contract\`" "$deployment_file"; then
    echo "::error file=${deployment_file}::deployable contract '${contract}' is missing from DEPLOYMENTS.md" >&2
    fail=1
  fi
done

if [ "$fail" -eq 0 ]; then
  echo "DEPLOYMENTS.md lists every deployable contract."
fi
exit "$fail"
