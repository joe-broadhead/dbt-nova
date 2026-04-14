#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/clean_cache.sh [options]

Clean dbt-nova cache directories.

Targets:
  --all           Clean manifests + instances + embeddings caches (default)
  --manifests     Clean manifest cache directory only
  --instances     Clean instance/index storage directory only
  --embeddings    Clean embeddings cache directory only

Path options:
  --storage-root <path>         Base storage root (default: ${DBT_NOVA_STORAGE_DIR:-.dbt-nova})
  --manifest-cache-dir <path>   Manifest cache path (default: <storage_root>/manifests)
  --embeddings-cache-dir <path> Embeddings cache path (default: ${DBT_NOVA_EMBEDDINGS_CACHE_DIR:-$HOME/.dbt-nova/.fastembed_cache})

Safety/options:
  --dry-run       Show what would be removed without deleting
  -y, --yes       Skip confirmation prompt
  -h, --help      Show this help
EOF
}

storage_root="${DBT_NOVA_STORAGE_DIR:-.dbt-nova}"
manifest_cache_dir=""
embeddings_cache_dir=""

clean_manifests=0
clean_instances=0
clean_embeddings=0
explicit_target=0

dry_run=0
assume_yes=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all)
      clean_manifests=1
      clean_instances=1
      clean_embeddings=1
      explicit_target=1
      ;;
    --manifests)
      clean_manifests=1
      explicit_target=1
      ;;
    --instances)
      clean_instances=1
      explicit_target=1
      ;;
    --embeddings)
      clean_embeddings=1
      explicit_target=1
      ;;
    --storage-root)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --storage-root" >&2
        exit 1
      fi
      storage_root="$2"
      shift
      ;;
    --manifest-cache-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --manifest-cache-dir" >&2
        exit 1
      fi
      manifest_cache_dir="$2"
      shift
      ;;
    --embeddings-cache-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --embeddings-cache-dir" >&2
        exit 1
      fi
      embeddings_cache_dir="$2"
      shift
      ;;
    --dry-run)
      dry_run=1
      ;;
    -y|--yes)
      assume_yes=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

if [[ $explicit_target -eq 0 ]]; then
  clean_manifests=1
  clean_instances=1
  clean_embeddings=1
fi

if [[ -z "$manifest_cache_dir" ]]; then
  manifest_cache_dir="${storage_root}/manifests"
fi
if [[ -z "$embeddings_cache_dir" ]]; then
  embeddings_cache_dir="${DBT_NOVA_EMBEDDINGS_CACHE_DIR:-${HOME:-.}/.dbt-nova/.fastembed_cache}"
fi
instances_dir="${storage_root}/instances"

labels=()
paths=()

add_target() {
  labels+=("$1")
  paths+=("$2")
}

if [[ $clean_manifests -eq 1 ]]; then
  add_target "manifest cache" "$manifest_cache_dir"
fi
if [[ $clean_instances -eq 1 ]]; then
  add_target "instance/index cache" "$instances_dir"
fi
if [[ $clean_embeddings -eq 1 ]]; then
  add_target "embeddings cache" "$embeddings_cache_dir"
fi

if [[ ${#paths[@]} -eq 0 ]]; then
  echo "No cache targets selected."
  exit 0
fi

cwd="$(pwd -P)"
home_dir="${HOME:-}"

assert_safe_delete_path() {
  local path="$1"
  local resolved="$path"

  if [[ -e "$path" ]]; then
    resolved="$(cd "$path" && pwd -P)"
  fi

  if [[ -z "$resolved" || "$resolved" == "/" || "$resolved" == "." ]]; then
    echo "Refusing to delete unsafe path: '$path'" >&2
    exit 1
  fi
  if [[ -n "$home_dir" && "$resolved" == "$home_dir" ]]; then
    echo "Refusing to delete HOME directory: '$path'" >&2
    exit 1
  fi
  if [[ "$resolved" == "$cwd" ]]; then
    echo "Refusing to delete current working directory: '$path'" >&2
    exit 1
  fi
}

echo "dbt-nova cache cleanup plan:"
for i in "${!paths[@]}"; do
  printf '  - %s: %s\n' "${labels[$i]}" "${paths[$i]}"
done
if [[ $dry_run -eq 1 ]]; then
  echo "Mode: dry-run (no files will be deleted)."
fi

if [[ $dry_run -eq 0 && $assume_yes -eq 0 ]]; then
  read -r -p "Proceed with deletion? [y/N]: " answer
  if [[ ! "${answer}" =~ ^[Yy]$ ]]; then
    echo "Cancelled."
    exit 0
  fi
fi

removed=0
skipped=0

for i in "${!paths[@]}"; do
  label="${labels[$i]}"
  path="${paths[$i]}"
  assert_safe_delete_path "$path"

  if [[ ! -e "$path" ]]; then
    printf 'skip: %s does not exist (%s)\n' "$label" "$path"
    skipped=$((skipped + 1))
    continue
  fi

  if [[ $dry_run -eq 1 ]]; then
    printf 'would remove: %s (%s)\n' "$label" "$path"
    continue
  fi

  rm -rf -- "$path"
  printf 'removed: %s (%s)\n' "$label" "$path"
  removed=$((removed + 1))
done

if [[ $dry_run -eq 1 ]]; then
  echo "Done (dry-run)."
else
  echo "Done. Removed $removed target(s), skipped $skipped target(s)."
fi
