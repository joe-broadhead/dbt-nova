#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/check_config_reference.sh [path]

Validate that docs/config_defaults.json matches current compiled defaults.

Optional path argument defaults to docs/config_defaults.json.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required to normalize JSON payloads for comparison." >&2
  exit 1
fi

expected_file="${1:-docs/config_defaults.json}"

if [[ ! -f "$expected_file" ]]; then
  echo "Expected config defaults file not found: $expected_file" >&2
  exit 1
fi

tmp_file="$(mktemp)"
normalized_expected="$(mktemp)"
normalized_actual="$(mktemp)"
diff_file="$(mktemp)"
trap 'rm -f "$tmp_file" "$normalized_expected" "$normalized_actual" "$diff_file"' EXIT

cargo run --locked --quiet -- config show --defaults --json > "$tmp_file"
python3 - "$expected_file" "$tmp_file" "$normalized_expected" "$normalized_actual" <<'PY'
import json
import sys

expected_path, actual_path, normalized_expected_path, normalized_actual_path = sys.argv[1:5]

with open(expected_path, "r", encoding="utf-8") as f:
    expected_json = json.load(f)
with open(actual_path, "r", encoding="utf-8") as f:
    envelope = json.load(f)

actual_json = envelope
if isinstance(envelope, dict) and "data" in envelope:
    actual_json = envelope["data"]

for path, data in [
    (normalized_expected_path, expected_json),
    (normalized_actual_path, actual_json),
]:
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, sort_keys=True, indent=2)
        f.write("\n")
PY

if ! diff -u "$normalized_expected" "$normalized_actual" > "$diff_file"; then
  echo "docs/config_defaults.json is out of sync with current defaults." >&2
  echo "Regenerate with: scripts/update_config_reference.sh" >&2
  sed -n '1,120p' "$diff_file" >&2
  echo "... (truncated)" >&2
  exit 1
fi
