#!/usr/bin/env bash
set -euo pipefail

WATCHLIST_FILE="${1:-dependency-watchlist.toml}"

if [[ ! -f "$WATCHLIST_FILE" ]]; then
  echo "dependency watchlist file not found: $WATCHLIST_FILE" >&2
  exit 1
fi

watchlist_dir="$(cd "$(dirname "$WATCHLIST_FILE")" && pwd)"
cargo_toml="${watchlist_dir}/Cargo.toml"
cargo_lock="${watchlist_dir}/Cargo.lock"

if [[ ! -f "$cargo_toml" ]]; then
  echo "missing Cargo.toml at $cargo_toml" >&2
  exit 1
fi
if [[ ! -f "$cargo_lock" ]]; then
  echo "missing Cargo.lock at $cargo_lock" >&2
  exit 1
fi

today="$(date +%F)"
found=0
failed=0

check_cargo_toml_has_ort_sys_rc_pin() {
  grep -Fq 'ort-sys = "=2.0.0-rc.4"' "$cargo_toml"
}

check_cargo_toml_has_reqwest_012_direct() {
  grep -Eq '^reqwest[[:space:]]*=[[:space:]]*\{[^}]*version[[:space:]]*=[[:space:]]*"0\.12"' "$cargo_toml"
}

check_cargo_lock_has_ort_sys_rc4() {
  awk '
    $0 == "name = \"ort-sys\"" { in_pkg = 1; next }
    in_pkg && /^version = "2\.0\.0-rc\.4"$/ { found = 1; exit }
    in_pkg && /^\[\[package\]\]/ { in_pkg = 0 }
    END { exit found ? 0 : 1 }
  ' "$cargo_lock"
}

check_cargo_lock_has_reqwest_011() {
  awk '
    $0 == "name = \"reqwest\"" { in_pkg = 1; next }
    in_pkg && /^version = "0\.11\./ { found = 1; exit }
    in_pkg && /^\[\[package\]\]/ { in_pkg = 0 }
    END { exit found ? 0 : 1 }
  ' "$cargo_lock"
}

check_cargo_lock_has_reqwest_012() {
  awk '
    $0 == "name = \"reqwest\"" { in_pkg = 1; next }
    in_pkg && /^version = "0\.12\./ { found = 1; exit }
    in_pkg && /^\[\[package\]\]/ { in_pkg = 0 }
    END { exit found ? 0 : 1 }
  ' "$cargo_lock"
}

run_state_check() {
  case "$1" in
    cargo_toml_has_ort_sys_rc_pin)
      check_cargo_toml_has_ort_sys_rc_pin
      ;;
    cargo_toml_has_reqwest_012_direct)
      check_cargo_toml_has_reqwest_012_direct
      ;;
    cargo_lock_has_ort_sys_rc4)
      check_cargo_lock_has_ort_sys_rc4
      ;;
    cargo_lock_has_reqwest_011)
      check_cargo_lock_has_reqwest_011
      ;;
    cargo_lock_has_reqwest_012)
      check_cargo_lock_has_reqwest_012
      ;;
    *)
      return 2
      ;;
  esac
}

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

reset_item() {
  item_id=""
  item_owner=""
  item_review_by=""
  item_summary=""
  item_current_state=""
  item_upgrade_trigger=""
  item_upgrade_plan=""
  item_state_checks=()
}

validate_item() {
  [[ -n "$item_id" ]] || return 0

  found=$((found + 1))
  local missing=()

  [[ -n "$item_owner" ]] || missing+=("owner")
  [[ -n "$item_review_by" ]] || missing+=("review_by")
  [[ -n "$item_summary" ]] || missing+=("summary")
  [[ -n "$item_current_state" ]] || missing+=("current_state")
  [[ -n "$item_upgrade_trigger" ]] || missing+=("upgrade_trigger")
  [[ -n "$item_upgrade_plan" ]] || missing+=("upgrade_plan")
  [[ ${#item_state_checks[@]} -gt 0 ]] || missing+=("state_checks")

  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "$item_id: missing required fields: ${missing[*]}" >&2
    failed=1
    return 0
  fi

  if [[ ! "$item_review_by" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    echo "$item_id: invalid review_by date: $item_review_by" >&2
    failed=1
    return 0
  fi

  if [[ "$item_review_by" < "$today" ]]; then
    echo "$item_id: review_by expired on $item_review_by (today: $today)" >&2
    failed=1
  fi

  local check_name
  for check_name in "${item_state_checks[@]}"; do
    if run_state_check "$check_name"; then
      continue
    fi

    if [[ $? -eq 2 ]]; then
      echo "$item_id: unknown state check '$check_name'" >&2
    else
      echo "$item_id: state check failed: $check_name. Update dependencies or refresh $(basename "$WATCHLIST_FILE")." >&2
    fi
    failed=1
  done
}

reset_item
while IFS= read -r raw_line || [[ -n "$raw_line" ]]; do
  line="$(trim "$raw_line")"
  [[ -z "$line" || "$line" == \#* ]] && continue

  if [[ "$line" == "[[item]]" ]]; then
    validate_item
    reset_item
    continue
  fi

  if [[ "$line" =~ ^([a-z_]+)[[:space:]]*=[[:space:]]*\"(.*)\"$ ]]; then
    key="${BASH_REMATCH[1]}"
    value="${BASH_REMATCH[2]}"
    case "$key" in
      id) item_id="$value" ;;
      owner) item_owner="$value" ;;
      review_by) item_review_by="$value" ;;
      summary) item_summary="$value" ;;
      current_state) item_current_state="$value" ;;
      upgrade_trigger) item_upgrade_trigger="$value" ;;
      upgrade_plan) item_upgrade_plan="$value" ;;
    esac
    continue
  fi

  if [[ "$line" =~ ^state_checks[[:space:]]*=[[:space:]]*\[(.*)\]$ ]]; then
    inner="${BASH_REMATCH[1]}"
    item_state_checks=()
    if [[ -n "${inner//[[:space:]]/}" ]]; then
      IFS=',' read -r -a checks <<<"$inner"
      for check_name in "${checks[@]}"; do
        check_name="$(trim "$check_name")"
        check_name="${check_name#\"}"
        check_name="${check_name%\"}"
        [[ -n "$check_name" ]] && item_state_checks+=("$check_name")
      done
    fi
    continue
  fi
done < "$WATCHLIST_FILE"

validate_item

if [[ $found -eq 0 ]]; then
  echo "no watchlist items found in $WATCHLIST_FILE"
  exit 0
fi

if [[ $failed -ne 0 ]]; then
  echo "dependency watchlist validation failed" >&2
  exit 1
fi

echo "dependency watchlist validation passed ($found entries, review date >= $today)"
