#!/usr/bin/env bash
# Fail on any drift between the committed error reference and generated output.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT
bash scripts/gen_error_docs.sh "${tmp}"

if ! diff -u docs/ERRORS.md "${tmp}"; then
  echo "error: docs/ERRORS.md is stale; run 'make docs-errors' and commit it" >&2
  exit 1
fi

echo "ok: docs/ERRORS.md matches generated errors.rs documentation"
