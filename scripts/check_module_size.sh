#!/usr/bin/env bash
set -euo pipefail

EXCEPTIONS_FILE="${1:-module-size-exceptions.tsv}"
SOFT_LIMIT="${DBT_NOVA_MODULE_SOFT_LIMIT:-1200}"
HARD_LIMIT="${DBT_NOVA_MODULE_HARD_LIMIT:-1800}"

if [[ ! -f "$EXCEPTIONS_FILE" ]]; then
  echo "module-size exceptions file not found: $EXCEPTIONS_FILE" >&2
  exit 1
fi

today="$(date +%F)"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

soft_report="$tmpdir/soft.tsv"
hard_report="$tmpdir/hard.tsv"
: >"$soft_report"
: >"$hard_report"

is_counted_path() {
  local path="$1"

  case "$path" in
    target/*|vendor/*|tests/fixtures/*|tests/snapshots/*)
      return 1
      ;;
    Cargo.lock|docs/config_defaults.json|schemas/nova/v0.json)
      return 1
      ;;
  esac

  case "$path" in
    *.rs|*.sh|*.py|*.md|*.yml|*.yaml)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

while IFS= read -r -d '' path; do
  is_counted_path "$path" || continue
  [[ -f "$path" ]] || continue

  lines="$(wc -l <"$path" | tr -d '[:space:]')"
  if [[ "$lines" -gt "$SOFT_LIMIT" ]]; then
    printf '%s\t%s\n' "$path" "$lines" >>"$soft_report"
  fi
  if [[ "$lines" -gt "$HARD_LIMIT" ]]; then
    printf '%s\t%s\n' "$path" "$lines" >>"$hard_report"
  fi
done < <(git ls-files -z)

sort -t $'\t' -k2,2nr -o "$soft_report" "$soft_report"
sort -t $'\t' -k2,2nr -o "$hard_report" "$hard_report"

hard_lines_for_path() {
  local path="$1"
  awk -F '\t' -v path="$path" '
    $1 == path { print $2; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$hard_report"
}

failed=0
registered=0
checked=0

while IFS= read -r raw_line || [[ -n "$raw_line" ]]; do
  [[ -z "$raw_line" || "$raw_line" == \#* ]] && continue

  IFS=$'\t' read -r path owner review_by reason extra <<<"$raw_line"
  registered=$((registered + 1))

  missing=()
  [[ -n "${path:-}" ]] || missing+=("path")
  [[ -n "${owner:-}" ]] || missing+=("owner")
  [[ -n "${review_by:-}" ]] || missing+=("review_by")
  [[ -n "${reason:-}" ]] || missing+=("reason")

  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "exception row $registered is invalid: missing ${missing[*]}" >&2
    failed=1
    continue
  fi
  if [[ -n "${extra:-}" ]]; then
    echo "$path: exception row has unexpected extra tab-separated fields" >&2
    failed=1
    continue
  fi

  if [[ ! -f "$path" ]]; then
    echo "$path: exception points to a missing file" >&2
    failed=1
    continue
  fi

  if [[ ! "$review_by" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    echo "$path: invalid review_by date: $review_by" >&2
    failed=1
    continue
  fi

  if [[ "$review_by" < "$today" ]]; then
    echo "$path: module-size exception expired on $review_by (today: $today)" >&2
    failed=1
  fi

  if ! hard_lines_for_path "$path" >/dev/null; then
    echo "$path: stale module-size exception; file is now at or below $HARD_LIMIT LOC" >&2
    failed=1
    continue
  fi

  checked=$((checked + 1))
done <"$EXCEPTIONS_FILE"

while IFS=$'\t' read -r path lines; do
  [[ -n "${path:-}" ]] || continue
  if ! awk -F '\t' -v path="$path" '$1 == path { found = 1 } END { exit found ? 0 : 1 }' "$EXCEPTIONS_FILE"; then
    echo "$path: $lines LOC exceeds hard limit $HARD_LIMIT and is not in $EXCEPTIONS_FILE" >&2
    failed=1
  fi
done <"$hard_report"

if [[ -s "$soft_report" ]]; then
  echo "files above soft target ($SOFT_LIMIT LOC):"
  awk -F '\t' '{ printf "  %5d %s\n", $2, $1 }' "$soft_report"
else
  echo "no files above soft target ($SOFT_LIMIT LOC)"
fi

if [[ $registered -eq 0 ]]; then
  echo "no module-size exceptions registered in $EXCEPTIONS_FILE" >&2
  failed=1
fi

if [[ $failed -ne 0 ]]; then
  echo "module-size ratchet check failed" >&2
  exit 1
fi

echo "module-size ratchet check passed ($checked hard-threshold exceptions, review date >= $today)"
