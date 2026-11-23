#!/usr/bin/env bash
# Regenerate the canonical error ABI table from Rust enum documentation.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-${ROOT}/docs/ERRORS.md}"

python3 - "${ROOT}/contracts/common/src/errors.rs" "${DEST}" <<'PY'
from pathlib import Path
import re
import sys

source = Path(sys.argv[1]).read_text(encoding="utf-8")
destination = Path(sys.argv[2])
try:
    enum = source.split("pub enum Error {", 1)[1].split("\n}", 1)[0]
except IndexError as exc:
    raise SystemExit("error: cannot find Error enum") from exc

owner = "generic"
docs = []
rows = []
for line in enum.splitlines():
    group = re.match(r"^\s*// ----- ([a-z_]+):", line)
    if group:
        owner = group.group(1)
        docs = []
        continue
    doc = re.match(r"^\s*///\s?(.*)$", line)
    if doc:
        docs.append(doc.group(1))
        continue
    variant = re.match(r"^\s*([A-Za-z]\w*)\s*=\s*(\d+),\s*$", line)
    if variant:
        # Join wrapped Rust doc-comment lines up to the first complete
        # sentence. Do not emit truncated descriptions when the first line
        # is only the beginning of a sentence.
        sentence = []
        for item in docs:
            if not item.strip():
                break
            sentence.append(item.strip())
            if item.rstrip().endswith((".", "!", "?")):
                break
        meaning = " ".join(sentence)
        if not meaning:
            raise SystemExit(f"error: {variant.group(1)} has no /// documentation")
        rows.append((int(variant.group(2)), variant.group(1), owner, meaning))
        docs = []
    elif line.strip() and not line.strip().startswith("//"):
        docs = []

if not rows:
    raise SystemExit("error: no Error variants found")

header = """# Error codes

The canonical mapping of `sororail_common::errors::Error` to stable ABI
numbers. The variants, owning contracts and descriptions below are generated
from `contracts/common/src/errors.rs` — do not edit the table by hand.

Run `make docs-errors` after modifying the enum or its documentation.
`make check-error-docs` verifies that this file matches the generated output.

| Code | Variant | Owner | Meaning |
|------|---------|-------|---------|
"""
footer = """
`102` (`BatchDuplicateRecipient`) was removed before any release and stays
burned rather than reused; see the comment at the end of `errors.rs`.
"""
table = "".join(
    f"| {code} | `{name}` | {owner} | {meaning.replace('|', chr(92) + '|')} |\n"
    for code, name, owner, meaning in rows
)
destination.write_text(header + table + footer, encoding="utf-8")
print(f"Generated {destination} ({len(rows)} error codes)")
PY
