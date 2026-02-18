#!/usr/bin/env bash
set -euo pipefail

REPO_SLUG="${DBT_NOVA_REPO:-joe-broadhead/dbt-nova}"
VERSION="${DBT_NOVA_VERSION:-latest}"
INSTALL_DIR="${DBT_NOVA_INSTALL_DIR:-$HOME/.local/bin}"
INSTALL_FLAVOR="${DBT_NOVA_INSTALL_FLAVOR:-}"
NON_INTERACTIVE="${DBT_NOVA_INSTALL_NONINTERACTIVE:-0}"
VERIFY_CHECKSUM="${DBT_NOVA_VERIFY_CHECKSUM:-1}"
VERIFY_SIGNATURE="${DBT_NOVA_VERIFY_SIGNATURE:-0}"
COSIGN_BINARY="${DBT_NOVA_COSIGN_BINARY:-cosign}"
DOWNLOAD_TOKEN="${DBT_NOVA_GITHUB_TOKEN:-${GITHUB_TOKEN:-${GH_TOKEN:-}}}"

usage() {
  cat <<'EOF'
Usage: install.sh [--bundled|--slim] [--non-interactive|-y] [--install-dir <path>]

Downloads and installs dbt-nova from GitHub releases.

Defaults:
  - flavor: bundled
  - install dir: $HOME/.local/bin

Environment overrides:
  DBT_NOVA_REPO                    GitHub repo slug (default: joe-broadhead/dbt-nova)
  DBT_NOVA_GITHUB_TOKEN            Optional token for downloading from private repos
  DBT_NOVA_VERSION                 Release tag (default: latest)
  DBT_NOVA_INSTALL_DIR             Install directory for dbt-nova
  DBT_NOVA_INSTALL_FLAVOR          bundled|slim
  DBT_NOVA_INSTALL_NONINTERACTIVE  1 to skip prompts (defaults to bundled)
  DBT_NOVA_VERIFY_CHECKSUM         1 to verify artifact checksum (default: 1)
  DBT_NOVA_VERIFY_SIGNATURE        1 to verify artifact signature (default: 0)
  DBT_NOVA_COSIGN_BINARY           Path to cosign executable (default: cosign)
EOF
}

compute_sha256() {
  local file="$1"

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    echo "Could not verify artifact hash: no sha256sum or shasum available." >&2
    return 1
  fi
}

expected_checksum() {
  local checksum_file="$1"
  local filename="$2"
  while read -r hash path _; do
    [[ -z "${hash}" || -z "${path}" ]] && continue
    path="${path#\*}"
    local candidate="${path##*/}"
    candidate="${candidate##*\\}"
    if [[ "${candidate}" == "${filename}" ]]; then
      printf '%s\n' "${hash}"
      return 0
    fi
  done < "${checksum_file}"
}

verify_checksum_file() {
  local artifact="$1"
  local checksum_file="$2"

  local expected
  local actual

  expected="$(expected_checksum "$checksum_file" "$(basename "$artifact")")"
  if [[ -z "$expected" ]]; then
    echo "Checksum file is missing entry for $(basename "$artifact")." >&2
    return 1
  fi

  actual="$(compute_sha256 "$artifact" | tr '[:upper:]' '[:lower:]')"
  expected="$(echo "$expected" | tr '[:upper:]' '[:lower:]')"

  if [[ "$actual" != "$expected" ]]; then
    echo "Checksum mismatch for $(basename "$artifact")." >&2
    echo "  Expected: $expected" >&2
    echo "  Actual:   $actual" >&2
    return 1
  fi
}

verify_signature() {
  local artifact="$1"
  local signature="$2"
  local certificate="$3"

  if [[ "${VERIFY_SIGNATURE}" != "1" ]]; then
    return 0
  fi

  if ! command -v "$COSIGN_BINARY" >/dev/null 2>&1; then
    echo "Signature verification requested but cosign is not available." >&2
    return 1
  fi

  if [[ ! -f "$signature" || ! -f "$certificate" ]]; then
    echo "Signature verification requested, but files are missing: $signature or $certificate." >&2
    return 1
  fi

  COSIGN_YES=1 "$COSIGN_BINARY" verify-blob \
    --signature "$signature" \
    --certificate "$certificate" \
    "$artifact"
}

download_file() {
  local file_name="$1"
  local url="$2"
  local out="$3"

  if [[ -n "${DOWNLOAD_TOKEN}" ]]; then
    if curl -fsSL -H "Authorization: Bearer ${DOWNLOAD_TOKEN}" "${url}" -o "${out}"; then
      return 0
    fi
  elif curl -fsSL "${url}" -o "${out}"; then
    return 0
  fi

  if command -v gh >/dev/null 2>&1; then
    echo "Direct download failed for ${file_name}; trying gh release download"
    if [[ "${VERSION}" == "latest" ]]; then
      gh release download --repo "${REPO_SLUG}" --pattern "${file_name}" --output "${out}"
    else
      gh release download "${VERSION}" --repo "${REPO_SLUG}" --pattern "${file_name}" --output "${out}"
    fi
    return 0
  fi

  echo "Download failed for ${file_name} and gh CLI is not available for fallback." >&2
  return 1
}

normalize_model_layout() {
  local models_root="$1"
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
    find "$models_root" -type f \
      -path "*/snapshots/*/model.onnx" \
      ! -path "*/snapshots/*/onnx/model.onnx" \
      -print0 2>/dev/null
  )

  if (( normalized > 0 )); then
    echo "Normalized ONNX layout for ${normalized} model snapshot(s)."
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundled)
      INSTALL_FLAVOR="bundled"
      ;;
    --slim)
      INSTALL_FLAVOR="slim"
      ;;
    --non-interactive|-y)
      NON_INTERACTIVE="1"
      ;;
    --install-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --install-dir" >&2
        exit 1
      fi
      INSTALL_DIR="$2"
      shift
      ;;
    --help|-h)
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

OS="$(uname -s)"
ARCH="$(uname -m)"

asset_os=""
case "${OS}" in
  Linux) asset_os="linux" ;;
  Darwin) asset_os="macos" ;;
  MINGW*|MSYS*|CYGWIN*) asset_os="windows" ;;
  *) echo "Unsupported OS: ${OS}" && exit 1 ;;
esac

case "${ARCH}" in
  x86_64|amd64) asset_arch="x86_64" ;;
  arm64|aarch64) asset_arch="arm64" ;;
  *) echo "Unsupported arch: ${ARCH}" && exit 1 ;;
esac

if [[ "${asset_os}" == "windows" ]]; then
  ext=".exe"
else
  ext=""
fi

if [[ -z "$INSTALL_FLAVOR" ]]; then
  if [[ "$NON_INTERACTIVE" == "1" || ! -t 0 ]]; then
    INSTALL_FLAVOR="bundled"
  else
    read -r -p "Install bundled artifact with pre-downloaded models? (Recommended) [Y/n]: " choice
    choice="${choice:-Y}"
    if [[ "${choice,,}" == "n" ]]; then
      INSTALL_FLAVOR="slim"
    else
      INSTALL_FLAVOR="bundled"
    fi
  fi
fi

if [[ "$INSTALL_FLAVOR" == "bundled" ]]; then
  flavor_suffix="-bundled"
elif [[ "$INSTALL_FLAVOR" == "slim" ]]; then
  flavor_suffix=""
else
  echo "Invalid flavor '${INSTALL_FLAVOR}'. Use bundled or slim." >&2
  exit 1
fi

asset="dbt-nova-${asset_os}-${asset_arch}${flavor_suffix}.tar.gz"
checksum_file="dbt-nova-${asset_os}-${asset_arch}.sha256"
signature_file="${checksum_file}.sig"
certificate_file="${checksum_file}.crt"
if [[ "${VERSION}" == "latest" ]]; then
  url="https://github.com/${REPO_SLUG}/releases/latest/download/${asset}"
  checksum_url="https://github.com/${REPO_SLUG}/releases/latest/download/${checksum_file}"
  signature_url="https://github.com/${REPO_SLUG}/releases/latest/download/${signature_file}"
  certificate_url="https://github.com/${REPO_SLUG}/releases/latest/download/${certificate_file}"
else
  url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${asset}"
  checksum_url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${checksum_file}"
  signature_url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${signature_file}"
  certificate_url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${certificate_file}"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

echo "Downloading ${url}"
download_file "${asset}" "${url}" "${tmp_dir}/${asset}"

if [[ "${VERIFY_CHECKSUM}" == "1" ]]; then
  echo "Downloading ${checksum_url}"
  download_file "${checksum_file}" "${checksum_url}" "${tmp_dir}/${checksum_file}"
  echo "Verifying SHA-256 checksum"
  verify_checksum_file "${tmp_dir}/${asset}" "${tmp_dir}/${checksum_file}"
fi

if [[ "${VERIFY_SIGNATURE}" == "1" ]]; then
  if [[ "${VERIFY_CHECKSUM}" != "1" ]]; then
    echo "Enabling checksum verification because signature verification requires checksum_file."
    echo "Downloading ${checksum_url}"
    download_file "${checksum_file}" "${checksum_url}" "${tmp_dir}/${checksum_file}"
    VERIFY_CHECKSUM=1
    echo "Verifying SHA-256 checksum"
    verify_checksum_file "${tmp_dir}/${asset}" "${tmp_dir}/${checksum_file}"
  fi

  echo "Downloading ${signature_url}"
  download_file "${signature_file}" "${signature_url}" "${tmp_dir}/${signature_file}"
  echo "Downloading ${certificate_url}"
  download_file "${certificate_file}" "${certificate_url}" "${tmp_dir}/${certificate_file}"
  verify_signature "${tmp_dir}/${checksum_file}" "${tmp_dir}/${signature_file}" "${tmp_dir}/${certificate_file}"
fi

tar -xzf "${tmp_dir}/${asset}" -C "${tmp_dir}"

mkdir -p "${INSTALL_DIR}"
cp "${tmp_dir}/dbt-nova${ext}" "${INSTALL_DIR}/dbt-nova${ext}"
chmod +x "${INSTALL_DIR}/dbt-nova${ext}"

if [[ "${INSTALL_FLAVOR}" == "bundled" && -d "${tmp_dir}/models" ]]; then
  models_dir="${INSTALL_DIR}/models"
  rm -rf "${models_dir}"
  mkdir -p "${models_dir}"
  cp -R "${tmp_dir}/models/." "${models_dir}/"
  normalize_model_layout "${models_dir}"
  echo "Models installed to ${models_dir}"
  echo "dbt-nova will auto-discover this colocated models/ directory."
elif [[ "${INSTALL_FLAVOR}" == "slim" ]]; then
  echo "Slim install selected. Models will be downloaded on first run."
  echo "Optional: pre-warm models with scripts/warm_models.sh after building from source."
fi

echo "Installed dbt-nova to ${INSTALL_DIR}/dbt-nova${ext}"
echo "Add ${INSTALL_DIR} to your PATH if needed."
