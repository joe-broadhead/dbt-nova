#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/check_skill_reference_maintenance.sh [options]

Warn or fail when dbt model/schema files change without matching skill,
reference, or eval maintenance evidence.

Options:
  --changed-files-file PATH  Newline-delimited changed files.
  --changed-files-json JSON  JSON array of changed files. Requires jq.
  --changed-file PATH        Add one changed file. May be repeated.
  --allowlist-file PATH      Newline-delimited glob patterns to suppress.
  --maintenance-glob GLOB    Extra maintenance evidence glob. May be repeated.
  --model-glob GLOB          Extra dbt model/schema glob. May be repeated.
  --mode advisory|required   Advisory exits 0 on missing evidence; required
                             exits 1. Defaults to advisory.
  --help                     Show this help.

Default dbt-change convention:
  models/**/*.sql, models/**/*.yml, models/**/*.yaml

Default maintenance evidence:
  .github/skills/**, evals/**, docs/domain-references/**, docs/domains/**,
  docs/features/domain-references.md
EOF
}

mode="advisory"
changed_files_file=""
changed_files_json=""
allowlist_file=""
changed_files=()
extra_model_globs=()
maintenance_globs=(
  ".github/skills/*"
  "evals/*"
  "docs/domain-references/*"
  "docs/domains/*"
  "docs/features/domain-references.md"
)
allowlist_globs=()

normalize_path() {
  local path="$1"
  path="${path//$'\r'/}"
  path="${path#./}"
  path="${path//\\//}"
  printf '%s' "$path"
}

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

append_changed_file() {
  local path
  path="$(normalize_path "$1")"
  [[ -n "$path" ]] && changed_files+=("$path")
}

read_changed_files_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "changed files file not found: $path" >&2
    exit 2
  fi
  local line
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="$(trim "$line")"
    [[ -z "$line" || "$line" == \#* ]] && continue
    append_changed_file "$line"
  done < "$path"
}

read_changed_files_json() {
  local payload="$1"
  if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for --changed-files-json" >&2
    exit 2
  fi
  local line
  while IFS= read -r line || [[ -n "$line" ]]; do
    append_changed_file "$line"
  done < <(jq -r '.[]' <<<"$payload")
}

read_allowlist_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "allowlist file not found: $path" >&2
    exit 2
  fi
  local line
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="$(trim "$line")"
    [[ -z "$line" || "$line" == \#* ]] && continue
    allowlist_globs+=("$(normalize_path "$line")")
  done < "$path"
}

matches_glob() {
  local path="$1"
  local pattern="$2"
  # shellcheck disable=SC2053 # RHS is an intentional glob pattern.
  [[ "$path" == $pattern ]]
}

matches_any_glob() {
  local path="$1"
  shift
  local pattern
  for pattern in "$@"; do
    if matches_glob "$path" "$pattern"; then
      return 0
    fi
  done
  return 1
}

is_default_dbt_model_or_schema_path() {
  local path="$1"
  [[ "$path" == models/* ]] || return 1
  case "${path##*.}" in
    sql|yml|yaml) return 0 ;;
    *) return 1 ;;
  esac
}

is_dbt_model_or_schema_path() {
  local path="$1"
  if is_default_dbt_model_or_schema_path "$path"; then
    return 0
  fi
  if [[ ${#extra_model_globs[@]} -gt 0 ]] && matches_any_glob "$path" "${extra_model_globs[@]}"; then
    return 0
  fi
  return 1
}

is_allowlisted() {
  local path="$1"
  [[ ${#allowlist_globs[@]} -gt 0 ]] || return 1
  matches_any_glob "$path" "${allowlist_globs[@]}"
}

is_maintenance_evidence() {
  local path="$1"
  matches_any_glob "$path" "${maintenance_globs[@]}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --changed-files-file)
      changed_files_file="${2:-}"
      [[ -n "$changed_files_file" ]] || { echo "--changed-files-file requires a path" >&2; exit 2; }
      shift 2
      ;;
    --changed-files-json)
      changed_files_json="${2:-}"
      [[ -n "$changed_files_json" ]] || { echo "--changed-files-json requires a JSON array" >&2; exit 2; }
      shift 2
      ;;
    --changed-file)
      [[ -n "${2:-}" ]] || { echo "--changed-file requires a path" >&2; exit 2; }
      append_changed_file "$2"
      shift 2
      ;;
    --allowlist-file)
      allowlist_file="${2:-}"
      [[ -n "$allowlist_file" ]] || { echo "--allowlist-file requires a path" >&2; exit 2; }
      shift 2
      ;;
    --maintenance-glob)
      [[ -n "${2:-}" ]] || { echo "--maintenance-glob requires a glob" >&2; exit 2; }
      maintenance_globs+=("$(normalize_path "$2")")
      shift 2
      ;;
    --model-glob)
      [[ -n "${2:-}" ]] || { echo "--model-glob requires a glob" >&2; exit 2; }
      extra_model_globs+=("$(normalize_path "$2")")
      shift 2
      ;;
    --mode)
      mode="${2:-}"
      case "$mode" in
        advisory|required) ;;
        *) echo "--mode must be advisory or required" >&2; exit 2 ;;
      esac
      shift 2
      ;;
    --advisory)
      mode="advisory"
      shift
      ;;
    --required)
      mode="required"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "$changed_files_file" ]] && read_changed_files_file "$changed_files_file"
[[ -n "$changed_files_json" ]] && read_changed_files_json "$changed_files_json"
[[ -n "$allowlist_file" ]] && read_allowlist_file "$allowlist_file"

if [[ ${#changed_files[@]} -eq 0 ]]; then
  echo "skill/reference maintenance check: no changed files supplied" >&2
  exit 2
fi

dbt_changes=()
suppressed_dbt_changes=()
maintenance_changes=()

for path in "${changed_files[@]}"; do
  if is_dbt_model_or_schema_path "$path"; then
    if is_allowlisted "$path"; then
      suppressed_dbt_changes+=("$path")
    else
      dbt_changes+=("$path")
    fi
  fi
  if is_maintenance_evidence "$path"; then
    maintenance_changes+=("$path")
  fi
done

if [[ ${#dbt_changes[@]} -eq 0 ]]; then
  if [[ ${#suppressed_dbt_changes[@]} -gt 0 ]]; then
    echo "skill/reference maintenance check: dbt model/schema changes are allowlisted"
  else
    echo "skill/reference maintenance check: no dbt model/schema changes found"
  fi
  exit 0
fi

if [[ ${#maintenance_changes[@]} -gt 0 ]]; then
  echo "skill/reference maintenance check: ok"
  echo "  dbt model/schema changes: ${#dbt_changes[@]}"
  echo "  maintenance evidence: ${#maintenance_changes[@]}"
  exit 0
fi

{
  echo "skill/reference maintenance check: dbt model/schema changes found without skill/reference/eval maintenance evidence"
  echo "  mode: $mode"
  echo "  dbt model/schema changes:"
  printf '    - %s\n' "${dbt_changes[@]}"
  echo "  expected at least one changed path matching:"
  printf '    - %s\n' "${maintenance_globs[@]}"
  if [[ ${#suppressed_dbt_changes[@]} -gt 0 ]]; then
    echo "  allowlisted dbt changes:"
    printf '    - %s\n' "${suppressed_dbt_changes[@]}"
  fi
  echo "  update the relevant skill, domain reference, or eval suite; or add an explicit allowlist entry when no maintenance is needed."
} >&2

if [[ "$mode" == "required" ]]; then
  exit 1
fi

exit 0
