#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_path="${DBT_NOVA_BIN:-$repo_root/target/release/dbt-nova}"
cache_dir="${DBT_NOVA_EMBEDDINGS_CACHE_DIR:-$HOME/.dbt-nova/.fastembed_cache}"
mode="${1:-partial}"
manifest_path="${DBT_NOVA_WARMUP_MANIFEST_PATH:-${DBT_MANIFEST_PATH:-}}"
required_models="${DBT_NOVA_WARMUP_REQUIRED_MODELS:-3}"
required_cache_files="${DBT_NOVA_WARMUP_REQUIRED_CACHE_FILES:-2}"
log_path="${DBT_NOVA_WARMUP_LOG_PATH:-/tmp/dbt-nova-warmup.log}"
repo_slug="${DBT_NOVA_WARMUP_REPO:-joe-broadhead/dbt-nova}"
release_version="${DBT_NOVA_WARMUP_VERSION:-latest}"
fallback_from_release="${DBT_NOVA_WARMUP_FALLBACK_FROM_RELEASE:-1}"
fallback_from_hf_direct="${DBT_NOVA_WARMUP_FALLBACK_FROM_HF_DIRECT:-1}"
checksum_mode="${DBT_NOVA_WARMUP_CHECKSUM_MODE:-warn}"
checksum_file="${DBT_NOVA_WARMUP_CHECKSUM_FILE:-}"
allow_mutable_hf_revisions="${DBT_NOVA_WARMUP_ALLOW_MUTABLE_HF_REVISIONS:-0}"
e5_revision="${DBT_NOVA_WARMUP_E5_REVISION:-d128750597153bb5987e10b1c3493a34e5a4502a}"
splade_revision="${DBT_NOVA_WARMUP_SPLADE_REVISION:-efcd182bc7eb351e81a9445752d4388c2bab500b}"
reranker_revision="${DBT_NOVA_WARMUP_RERANKER_REVISION:-9cfeff2df7d40d1b78e75e5e9cebec92a99813c9}"
pid=""

usage() {
  cat <<'EOF'
Usage: scripts/warm_models.sh [partial|full]

Pre-download vector/sparse/reranker model files into a cache directory.
Default mode is 'partial'.

Modes:
  partial  Only ensure model files are present in cache (default)
  full     Ensure model files, then run `dbt-nova manifest warm --vector --sparse --reranker` to generate semantic caches

Optional env overrides:
  DBT_NOVA_BIN                         Path to dbt-nova binary
  DBT_NOVA_EMBEDDINGS_CACHE_DIR        Cache directory (default: \$HOME/.dbt-nova/.fastembed_cache)
  DBT_NOVA_WARMUP_MANIFEST_PATH        Manifest file for full mode (required)
  DBT_NOVA_WARMUP_REQUIRED_MODELS      Required distinct model snapshot count (default: 3)
                                       Direct HF fallback seeds models in priority order
                                       (embedding -> sparse -> reranker) up to this count.
  DBT_NOVA_WARMUP_REQUIRED_CACHE_FILES Required embedding cache files in full mode (default: 2)
  DBT_NOVA_WARMUP_LOG_PATH             Log file path (default: /tmp/dbt-nova-warmup.log)
  DBT_NOVA_WARMUP_REPO                 GitHub repo slug for fallback bundle
  DBT_NOVA_WARMUP_VERSION              Release tag for fallback bundle (default: latest)
  DBT_NOVA_WARMUP_FALLBACK_FROM_RELEASE
                                       1 to seed from bundled release on warmup failure (default: 1)
  DBT_NOVA_WARMUP_FALLBACK_FROM_HF_DIRECT
                                       1 to seed cache from direct Hugging Face downloads on warmup failure (default: 1)
  DBT_NOVA_WARMUP_CHECKSUM_MODE        Checksum policy for direct HF fallback downloads:
                                       off | warn (default) | required
  DBT_NOVA_WARMUP_CHECKSUM_FILE        Path to checksum manifest for direct HF fallback.
                                       Format: "<sha256> <url>" (space-delimited)
  DBT_NOVA_WARMUP_E5_REVISION          Pinned intfloat/multilingual-e5-base revision
  DBT_NOVA_WARMUP_SPLADE_REVISION      Pinned Qdrant/Splade_PP_en_v1 revision
  DBT_NOVA_WARMUP_RERANKER_REVISION    Pinned jina reranker revision
  DBT_NOVA_WARMUP_ALLOW_MUTABLE_HF_REVISIONS
                                       1 to allow mutable revisions such as main/master/latest (default: 0)

Example:
  # Model files only (default)
  DBT_NOVA_EMBEDDINGS_CACHE_DIR="\$HOME/.dbt-nova/.fastembed_cache" scripts/warm_models.sh

  # Model files + manifest-scoped semantic caches
  DBT_NOVA_WARMUP_MANIFEST_PATH=/path/to/manifest.json scripts/warm_models.sh full
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ "$mode" != "partial" && "$mode" != "full" ]]; then
  echo "Invalid mode '$mode'. Expected 'partial' or 'full'." >&2
  exit 1
fi

if [[ $# -gt 1 ]]; then
  echo "Too many arguments. Usage: scripts/warm_models.sh [partial|full]" >&2
  exit 1
fi

if [[ "$checksum_mode" != "off" && "$checksum_mode" != "warn" && "$checksum_mode" != "required" ]]; then
  echo "Invalid DBT_NOVA_WARMUP_CHECKSUM_MODE '$checksum_mode'. Expected off|warn|required." >&2
  exit 1
fi

if [[ -n "$checksum_file" && ! -f "$checksum_file" ]]; then
  echo "Checksum manifest not found: $checksum_file" >&2
  exit 1
fi

if [[ "$checksum_mode" == "required" && -z "$checksum_file" ]]; then
  echo "DBT_NOVA_WARMUP_CHECKSUM_MODE=required requires DBT_NOVA_WARMUP_CHECKSUM_FILE." >&2
  exit 1
fi

is_mutable_hf_revision() {
  case "$1" in
    main|master|latest) return 0 ;;
    *) return 1 ;;
  esac
}

validate_hf_revision() {
  local name="$1"
  local revision="$2"
  if [[ -z "$revision" ]]; then
    echo "$name revision cannot be empty." >&2
    exit 1
  fi
  if [[ "$fallback_from_hf_direct" == "1" && "$allow_mutable_hf_revisions" != "1" ]] \
    && is_mutable_hf_revision "$revision"; then
    echo "$name revision '$revision' is mutable. Set a commit SHA or DBT_NOVA_WARMUP_ALLOW_MUTABLE_HF_REVISIONS=1 to opt in." >&2
    exit 1
  fi
}

validate_hf_revision "DBT_NOVA_WARMUP_E5_REVISION" "$e5_revision"
validate_hf_revision "DBT_NOVA_WARMUP_SPLADE_REVISION" "$splade_revision"
validate_hf_revision "DBT_NOVA_WARMUP_RERANKER_REVISION" "$reranker_revision"

cleanup() {
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

list_model_snapshots() {
  {
    find "$cache_dir" -type f -path "*/snapshots/*/onnx/model.onnx" 2>/dev/null \
      | sed -E 's#/onnx/model\.onnx$##'
    find "$cache_dir" -type f -path "*/snapshots/*/model.onnx" ! -path "*/snapshots/*/onnx/model.onnx" 2>/dev/null \
      | sed -E 's#/model\.onnx$##'
  } | sort -u
}

count_model_files() {
  list_model_snapshots | sed '/^$/d' | wc -l | tr -d ' '
}

normalize_onnx_layout() {
  local normalized=0
  local source_file
  local snapshot_dir
  local onnx_dir
  local target_file

  while IFS= read -r -d '' source_file; do
    snapshot_dir="$(dirname "$source_file")"
    onnx_dir="$snapshot_dir/onnx"
    target_file="$onnx_dir/model.onnx"

    if [[ -f "$target_file" ]]; then
      continue
    fi

    mkdir -p "$onnx_dir"
    cp "$source_file" "$target_file"
    normalized=$((normalized + 1))
  done < <(
    find "$cache_dir" -type f \
      -path "*/snapshots/*/model.onnx" \
      ! -path "*/snapshots/*/onnx/model.onnx" \
      -print0 2>/dev/null
  )

  if (( normalized > 0 )); then
    echo "Normalized ONNX layout for $normalized model snapshot(s)."
  fi
}

extract_warm_cache_paths() {
  local log_file="$1"
  grep -Eo '"(vector|sparse)"[[:space:]]*:[[:space:]]*"[^"]+"' "$log_file" 2>/dev/null \
    | sed -E 's/^"(vector|sparse)"[[:space:]]*:[[:space:]]*"([^"]+)"$/\2/' \
    | sort -u
}

verify_warm_cache_outputs() {
  local log_file="$1"
  local found=0
  local cache_path

  while IFS= read -r cache_path; do
    [[ -z "$cache_path" ]] && continue
    if [[ ! -f "$cache_path" ]]; then
      echo "Warmup failed (full): expected cache file is missing: $cache_path" >&2
      return 1
    fi
    found=$((found + 1))
  done < <(extract_warm_cache_paths "$log_file")

  if (( found < required_cache_files )); then
    echo "Warmup failed (full): expected >= $required_cache_files manifest-scoped semantic cache files, found $found" >&2
    return 1
  fi

  echo "$found"
}

get_content_length() {
  local url="$1"
  curl -sSIL --max-time 120 --retry 8 --retry-delay 2 --retry-all-errors "$url" \
    | tr -d '\r' \
    | awk 'tolower($1)=="content-length:"{v=$2} END{print v+0}'
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print tolower($1)}'
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print tolower($1)}'
    return 0
  fi
  if command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$path" | awk -F'=' '{gsub(/^[[:space:]]+/, "", $2); print tolower($2)}'
    return 0
  fi
  echo "Checksum verification requires one of: sha256sum, shasum, openssl." >&2
  return 1
}

lookup_expected_checksum() {
  local url="$1"
  local file="$2"
  awk -v target="$url" '
    /^[[:space:]]*#/ { next }
    NF < 2 { next }
    {
      hash=$1
      $1=""
      sub(/^[[:space:]]+/, "", $0)
      gsub(/[[:space:]]+$/, "", $0)
      if ($0 == target) {
        print tolower(hash)
        found=1
        exit 0
      }
    }
    END { if (!found) exit 1 }
  ' "$file"
}

verify_direct_hf_checksum() {
  local url="$1"
  local path="$2"
  local expected actual

  if [[ "$checksum_mode" == "off" ]]; then
    return 0
  fi
  if [[ -z "$checksum_file" ]]; then
    if [[ "$checksum_mode" == "required" ]]; then
      echo "Checksum verification required but no checksum manifest was configured." >&2
      return 3
    fi
    echo "Checksum verification warning: no checksum manifest configured; skipping $url." >&2
    return 0
  fi

  if ! expected="$(lookup_expected_checksum "$url" "$checksum_file")"; then
    if [[ "$checksum_mode" == "required" ]]; then
      echo "Checksum verification failed: no checksum entry for $url in $checksum_file" >&2
      return 3
    fi
    echo "Checksum verification warning: no checksum entry for $url; skipping." >&2
    return 0
  fi

  if ! actual="$(sha256_file "$path")"; then
    return 4
  fi
  if [[ "$actual" != "$expected" ]]; then
    echo "Checksum mismatch for $url (expected $expected, got $actual)." >&2
    return 2
  fi
  return 0
}

download_hf_file() {
  local url="$1"
  local target="$2"
  local expected actual tmp verify_status

  expected="$(get_content_length "$url")"
  mkdir -p "$(dirname "$target")"

  if [[ -f "$target" && "$expected" -gt 0 ]]; then
    actual="$(wc -c < "$target" | tr -d ' ')"
    if [[ "$actual" == "$expected" ]]; then
      if verify_direct_hf_checksum "$url" "$target"; then
        return 0
      else
        verify_status=$?
      fi
      if [[ "$verify_status" -eq 2 ]]; then
        echo "Cached file failed checksum verification; re-downloading: $target" >&2
        rm -f "$target"
      else
        return "$verify_status"
      fi
    fi
  fi

  tmp="${target}.tmp"
  if ! curl -fL --retry 10 --retry-delay 2 --retry-all-errors --continue-at - "$url" -o "$tmp"; then
    return 1
  fi

  if [[ "$expected" -gt 0 ]]; then
    actual="$(wc -c < "$tmp" | tr -d ' ')"
    if [[ "$actual" != "$expected" ]]; then
      echo "Download size mismatch for $url (expected $expected bytes, got $actual bytes)." >&2
      rm -f "$tmp"
      return 1
    fi
  fi

  if ! verify_direct_hf_checksum "$url" "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  mv "$tmp" "$target"
}

seed_from_bundled_release() {
  local os arch asset_os asset_arch asset url tmp_dir found
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux) asset_os="linux" ;;
    Darwin) asset_os="macos" ;;
    MINGW*|MSYS*|CYGWIN*) asset_os="windows" ;;
    *)
      echo "Fallback skipped: unsupported OS '$os'" >&2
      return 1
      ;;
  esac

  case "$arch" in
    x86_64|amd64) asset_arch="x86_64" ;;
    arm64|aarch64) asset_arch="arm64" ;;
    *)
      echo "Fallback skipped: unsupported architecture '$arch'" >&2
      return 1
      ;;
  esac

  asset="dbt-nova-${asset_os}-${asset_arch}-bundled.tar.gz"
  if [[ "$release_version" == "latest" ]]; then
    url="https://github.com/${repo_slug}/releases/latest/download/${asset}"
  else
    url="https://github.com/${repo_slug}/releases/download/${release_version}/${asset}"
  fi

  tmp_dir="$(mktemp -d)"
  echo "Attempting fallback seed from bundled release:"
  echo "  $url"
  if ! curl -fsSL "$url" -o "$tmp_dir/$asset"; then
    rm -rf "$tmp_dir"
    echo "Fallback failed: could not download bundled artifact." >&2
    return 1
  fi

  if ! tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"; then
    rm -rf "$tmp_dir"
    echo "Fallback failed: could not unpack bundled artifact." >&2
    return 1
  fi

  if [[ ! -d "$tmp_dir/models" ]]; then
    rm -rf "$tmp_dir"
    echo "Fallback failed: bundled artifact did not contain models/." >&2
    return 1
  fi

  cp -R "$tmp_dir/models/." "$cache_dir/"
  rm -rf "$tmp_dir"

  found="$(count_model_files)"
  if (( found < required_models )); then
    echo "Fallback failed: expected >= $required_models model.onnx files, found $found after seeding." >&2
    return 1
  fi

  echo "Fallback seed complete. Found $found model file(s) in $cache_dir."
  return 0
}

seed_repo_snapshot_from_hf() {
  local repo="$1"
  local revision="$2"
  shift 2
  local repo_dir ref_dir snap_dir file url target

  repo_dir="$cache_dir/models--${repo//\//--}"
  ref_dir="$repo_dir/refs"
  snap_dir="$repo_dir/snapshots/$revision"

  mkdir -p "$ref_dir" "$snap_dir"
  # hf-hub reads refs as raw bytes and does not trim, so refs/main must not end with newline.
  printf '%s' "$revision" > "$ref_dir/main"

  for file in "$@"; do
    url="https://huggingface.co/${repo}/resolve/${revision}/${file}"
    target="$snap_dir/$file"
    download_hf_file "$url" "$target"
  done
}

seed_from_hf_direct() {
  local found

  echo "Attempting fallback seed from direct Hugging Face downloads..."
  if ! command -v curl >/dev/null 2>&1; then
    echo "Fallback failed: curl is required for direct Hugging Face seeding." >&2
    return 1
  fi

  if (( required_models >= 1 )); then
    if ! seed_repo_snapshot_from_hf "intfloat/multilingual-e5-base" "$e5_revision" \
      "onnx/model.onnx" \
      "tokenizer.json" \
      "config.json" \
      "special_tokens_map.json" \
      "tokenizer_config.json"; then
      echo "Fallback failed while seeding intfloat/multilingual-e5-base." >&2
      return 1
    fi
  fi

  if (( required_models >= 2 )); then
    if ! seed_repo_snapshot_from_hf "Qdrant/Splade_PP_en_v1" "$splade_revision" \
      "model.onnx" \
      "tokenizer.json" \
      "config.json" \
      "special_tokens_map.json" \
      "tokenizer_config.json"; then
      echo "Fallback failed while seeding Qdrant/Splade_PP_en_v1." >&2
      return 1
    fi
  fi

  if (( required_models >= 3 )); then
    if ! seed_repo_snapshot_from_hf "jinaai/jina-reranker-v2-base-multilingual" "$reranker_revision" \
      "onnx/model.onnx" \
      "tokenizer.json" \
      "config.json" \
      "special_tokens_map.json" \
      "tokenizer_config.json"; then
      echo "Fallback failed while seeding jinaai/jina-reranker-v2-base-multilingual." >&2
      return 1
    fi
  fi

  found="$(count_model_files)"
  if (( found < required_models )); then
    echo "Fallback failed: expected >= $required_models model.onnx files, found $found after direct HF seeding." >&2
    return 1
  fi

  echo "Direct HF seed complete. Found $found model file(s) in $cache_dir."
  return 0
}

ensure_model_files() {
  local downloaded
  downloaded="$(count_model_files)"

  if (( downloaded >= required_models )); then
    return 0
  fi

  if [[ "$fallback_from_hf_direct" == "1" ]]; then
    echo "Model warmup incomplete (found $downloaded model files)."
    echo "Trying direct Hugging Face cache seeding..."
    if seed_from_hf_direct; then
      downloaded="$(count_model_files)"
    else
      echo "Direct Hugging Face cache seeding did not succeed." >&2
    fi
  fi

  if (( downloaded < required_models )) && [[ "$fallback_from_release" == "1" ]]; then
    echo "Model warmup still incomplete (found $downloaded model files)."
    echo "Trying bundled-release fallback..."
    if seed_from_bundled_release; then
      downloaded="$(count_model_files)"
    else
      echo "Bundled-release fallback did not succeed." >&2
    fi
  fi

  if (( downloaded < required_models )); then
    echo "Warmup failed: expected >= $required_models model.onnx files, found $downloaded" >&2
    return 1
  fi

  return 0
}

mkdir -p "$cache_dir"

echo "Starting warmup (mode: $mode)..."
echo "  cache:  $cache_dir"

if ! ensure_model_files; then
  exit 1
fi

normalize_onnx_layout

if [[ "$mode" == "partial" ]]; then
  downloaded="$(count_model_files)"
  echo "Warmup complete (partial). Found $downloaded model snapshot(s):"
  list_model_snapshots
  exit 0
fi

if [[ ! -x "$bin_path" ]]; then
  echo "Binary not found or not executable: $bin_path" >&2
  echo "Build first: cargo build --release --locked" >&2
  exit 1
fi

if [[ -z "$manifest_path" ]]; then
  echo "Full mode requires DBT_NOVA_WARMUP_MANIFEST_PATH (or DBT_MANIFEST_PATH)." >&2
  exit 1
fi

if [[ ! -f "$manifest_path" ]]; then
  echo "Manifest file not found: $manifest_path" >&2
  echo "Set DBT_NOVA_WARMUP_MANIFEST_PATH for full mode." >&2
  exit 1
fi

rm -f "$log_path"
echo "  binary: $bin_path"
echo "  manifest: $manifest_path"
echo "  logs:   $log_path"

if ! DBT_MANIFEST_PATH="$manifest_path" \
  DBT_NOVA_MANIFEST_REFRESH_SECS=0 \
  DBT_NOVA_EMBEDDINGS_CACHE_DIR="$cache_dir" \
  DBT_NOVA_SEARCH_ENABLE_VECTOR=true \
  DBT_NOVA_SEARCH_ENABLE_SPARSE=true \
  DBT_NOVA_SEARCH_ENABLE_RERANKER=true \
  DBT_NOVA_SEARCH_COLD_START_POLICY=build \
  DBT_NOVA_LOG="${DBT_NOVA_LOG:-info}" \
  "$bin_path" manifest warm --manifest-path "$manifest_path" --vector --sparse --reranker --json >"$log_path" 2>&1; then
  echo "Warmup failed (full)." >&2
  echo "Last log lines:" >&2
  tail -n 80 "$log_path" >&2 || true
  exit 1
fi

downloaded="$(count_model_files)"
cache_files="$(verify_warm_cache_outputs "$log_path")" || {
  echo "Last log lines:" >&2
  tail -n 80 "$log_path" >&2 || true
  exit 1
}

echo "Warmup complete (full). Found $downloaded model file(s)."
echo "Manifest-scoped semantic cache files ($cache_files):"
extract_warm_cache_paths "$log_path"
