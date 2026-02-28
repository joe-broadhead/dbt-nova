#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

tmp_file="$(mktemp)"
trap 'rm -f "$tmp_file"' EXIT

cargo run --locked --quiet -- config show --defaults --json > "$tmp_file"
python3 - "$tmp_file" <<'PY' > docs/config_defaults.json
import json
import sys

payload_path = sys.argv[1]

with open(payload_path, "r", encoding="utf-8") as f:
    payload = json.load(f)

data = payload.get("data") if isinstance(payload, dict) else None
if data is None:
    raise SystemExit("missing `data` field in config show --defaults --json output")

json.dump(data, sys.stdout, indent=2)
sys.stdout.write("\n")
PY
