#!/usr/bin/env bash
set -euo pipefail

DENY_TOML="${1:-deny.toml}"

if [[ ! -f "$DENY_TOML" ]]; then
  echo "deny file not found: $DENY_TOML" >&2
  exit 1
fi

today="$(date +%F)"
found=0
failed=0

while IFS= read -r line; do
  [[ "$line" == *"id = \"RUSTSEC-"* ]] || continue

  id="$(sed -nE 's/.*id = "([^"]+)".*/\1/p' <<<"$line")"
  reason="$(sed -nE 's/.*reason = "([^"]+)".*/\1/p' <<<"$line")"

  if [[ -z "$id" || -z "$reason" ]]; then
    echo "invalid ignore entry format (expected one-line id+reason object): $line" >&2
    failed=1
    continue
  fi

  found=$((found + 1))

  owner="$(sed -nE 's/.*owner=([^;]+).*/\1/p' <<<"$reason" | xargs)"
  review_by="$(sed -nE 's/.*review_by=([0-9]{4}-[0-9]{2}-[0-9]{2}).*/\1/p' <<<"$reason")"

  if [[ -z "$owner" ]]; then
    echo "$id is missing owner metadata in deny.toml reason" >&2
    failed=1
  fi
  if [[ -z "$review_by" ]]; then
    echo "$id is missing review_by metadata in deny.toml reason" >&2
    failed=1
    continue
  fi

  if [[ "$review_by" < "$today" ]]; then
    echo "$id ignore expired on $review_by (today: $today)" >&2
    failed=1
  fi
done <"$DENY_TOML"

if [[ $found -eq 0 ]]; then
  echo "no advisory ignores found in $DENY_TOML"
  exit 0
fi

if [[ $failed -ne 0 ]]; then
  echo "advisory ignore metadata check failed" >&2
  exit 1
fi

echo "advisory ignore metadata check passed ($found entries, review date >= $today)"
