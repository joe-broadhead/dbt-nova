#!/usr/bin/env bash
set -euo pipefail

REPO_SLUG="${DBT_NOVA_REPO:-joe-broadhead/dbt-nova}"
VERSION="${DBT_NOVA_VERSION:-latest}"
INSTALL_DIR="${DBT_NOVA_INSTALL_DIR:-$HOME/.local/bin}"
INSTALL_FLAVOR="${DBT_NOVA_INSTALL_FLAVOR:-}"
INSTALL_SKILLS="${DBT_NOVA_INSTALL_SKILLS:-0}"
SKILLS_DIR="${DBT_NOVA_SKILLS_DIR:-$HOME/.agents/skills}"
SKILLS_BUNDLE="${DBT_NOVA_SKILLS_BUNDLE:-}"
SKILL_NAME="${DBT_NOVA_SKILL_NAME:-}"
NON_INTERACTIVE="${DBT_NOVA_INSTALL_NONINTERACTIVE:-0}"
INSTALL_WARM_MODELS="${DBT_NOVA_INSTALL_WARM_MODELS:-0}"
VERIFY_CHECKSUM="${DBT_NOVA_VERIFY_CHECKSUM:-1}"
VERIFY_SIGNATURE="${DBT_NOVA_VERIFY_SIGNATURE:-1}"
COSIGN_BINARY="${DBT_NOVA_COSIGN_BINARY:-cosign}"
COSIGN_CERT_IDENTITY_REGEXP="${DBT_NOVA_COSIGN_CERT_IDENTITY_REGEXP:-https://github.com/${REPO_SLUG}/.github/workflows/release.yml@refs/tags/v.*}"
COSIGN_CERT_OIDC_ISSUER="${DBT_NOVA_COSIGN_CERT_OIDC_ISSUER:-https://token.actions.githubusercontent.com}"
DOWNLOAD_TOKEN="${DBT_NOVA_GITHUB_TOKEN:-${GITHUB_TOKEN:-${GH_TOKEN:-}}}"

usage() {
  cat <<'EOF'
Usage: install.sh [--bundled|--slim] [--warm-models] [--install-skills [--skill <name>]] [--skills-dir <path>] [--non-interactive|-y] [--install-dir <path>]

Downloads and installs dbt-nova from GitHub releases.

Defaults:
  - flavor: slim
  - install dir: $HOME/.local/bin

Environment overrides:
  DBT_NOVA_REPO                    GitHub repo slug (default: joe-broadhead/dbt-nova)
  DBT_NOVA_GITHUB_TOKEN            Optional token for downloading from private repos
  DBT_NOVA_VERSION                 Release tag (default: latest)
  DBT_NOVA_INSTALL_DIR             Install directory for dbt-nova
  DBT_NOVA_INSTALL_FLAVOR          bundled|slim
  DBT_NOVA_INSTALL_SKILLS          1 to install Agent Skills (default: 0)
  DBT_NOVA_SKILLS_DIR              Skills destination (default: $HOME/.agents/skills)
  DBT_NOVA_SKILL_NAME              Optional single standalone skill to install
  DBT_NOVA_SKILLS_BUNDLE           Deprecated cli|mcp compatibility selector
  DBT_NOVA_INSTALL_WARM_MODELS     1 to pre-warm model files after install (default: 0)
  DBT_NOVA_INSTALL_NONINTERACTIVE  1 to skip prompts (defaults to slim)
  DBT_NOVA_VERIFY_CHECKSUM         1 to verify artifact checksum (default: 1)
  DBT_NOVA_VERIFY_SIGNATURE        1|auto|0 checksum signature verification (default: 1)
  DBT_NOVA_COSIGN_BINARY           Path to cosign executable (default: cosign)
  DBT_NOVA_COSIGN_CERT_IDENTITY_REGEXP  Expected signing identity regexp
  DBT_NOVA_COSIGN_CERT_OIDC_ISSUER      Expected signing OIDC issuer
EOF
}

validate_skills_bundle() {
  case "$1" in
    cli|mcp) ;;
    *)
      echo "Invalid skills bundle '$1'. Use 'cli' or 'mcp'." >&2
      return 1
      ;;
  esac
}

validate_skill_name_segment() {
  local skill_name="$1"
  if [[ -z "${skill_name}" ]]; then
    echo "Skill name cannot be empty." >&2
    return 1
  fi
  if ! [[ "${skill_name}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
    echo "Invalid skill name '${skill_name}'. Use a single safe path segment." >&2
    return 1
  fi
}

resolve_skill_install_selection() {
  if [[ "${INSTALL_SKILLS}" != "1" ]]; then
    return 0
  fi

  if [[ -n "${SKILLS_BUNDLE}" ]]; then
    validate_skills_bundle "${SKILLS_BUNDLE}"
  fi

  if [[ -n "${SKILL_NAME}" ]]; then
    validate_skill_name_segment "${SKILL_NAME}"
  fi
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

  if [[ "${VERIFY_SIGNATURE}" == "0" ]]; then
    return 0
  fi

  if ! command -v "$COSIGN_BINARY" >/dev/null 2>&1; then
    if [[ "${VERIFY_SIGNATURE}" == "auto" ]]; then
      echo "cosign is not available; skipping automatic signature verification." >&2
      return 0
    fi
    echo "Signature verification is required but cosign is not available." >&2
    return 1
  fi

  if [[ ! -f "$signature" || ! -f "$certificate" ]]; then
    if [[ "${VERIFY_SIGNATURE}" == "auto" ]]; then
      echo "Signature files are unavailable; skipping automatic signature verification." >&2
      return 0
    fi
    echo "Signature verification is required, but files are missing: $signature or $certificate." >&2
    return 1
  fi

  COSIGN_YES=1 "$COSIGN_BINARY" verify-blob \
    --signature "$signature" \
    --certificate "$certificate" \
    --certificate-identity-regexp "$COSIGN_CERT_IDENTITY_REGEXP" \
    --certificate-oidc-issuer "$COSIGN_CERT_OIDC_ISSUER" \
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
    gh release download "${VERSION}" --repo "${REPO_SLUG}" --pattern "${file_name}" --output "${out}"
    return 0
  fi

  echo "Download failed for ${file_name} and gh CLI is not available for fallback." >&2
  return 1
}

download_raw_script() {
  local url="$1"
  local out="$2"
  if [[ -n "${DOWNLOAD_TOKEN}" ]]; then
    curl -fsSL -H "Authorization: Bearer ${DOWNLOAD_TOKEN}" "${url}" -o "${out}"
  else
    curl -fsSL "${url}" -o "${out}"
  fi
}

download_repo_archive() {
  local ref="$1"
  local out="$2"
  local archive_url="https://api.github.com/repos/${REPO_SLUG}/tarball/${ref}"
  if [[ -n "${DOWNLOAD_TOKEN}" ]]; then
    curl -fsSL \
      -H "Authorization: Bearer ${DOWNLOAD_TOKEN}" \
      -H "Accept: application/vnd.github+json" \
      "${archive_url}" \
      -o "${out}"
  else
    curl -fsSL \
      -H "Accept: application/vnd.github+json" \
      "${archive_url}" \
      -o "${out}"
  fi
}

latest_release_tag() {
  local api_url="https://api.github.com/repos/${REPO_SLUG}/releases/latest"
  local response=""

  if [[ -n "${DOWNLOAD_TOKEN}" ]]; then
    response="$(curl -fsSL -H "Authorization: Bearer ${DOWNLOAD_TOKEN}" "${api_url}" 2>/dev/null || true)"
  else
    response="$(curl -fsSL "${api_url}" 2>/dev/null || true)"
  fi

  if [[ -n "${response}" ]]; then
    printf '%s' "${response}" \
      | tr -d '\n' \
      | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
    return 0
  fi

  if command -v gh >/dev/null 2>&1; then
    gh release view --repo "${REPO_SLUG}" --json tagName --jq '.tagName' 2>/dev/null || true
  fi
}

resolve_latest_version() {
  if [[ "${VERSION}" != "latest" ]]; then
    return 0
  fi

  local tag
  tag="$(latest_release_tag)"
  if [[ -z "${tag}" ]]; then
    echo "Could not resolve the latest release tag for ${REPO_SLUG}." >&2
    echo "Set DBT_NOVA_VERSION to an explicit release tag." >&2
    exit 1
  fi
  VERSION="${tag}"
  echo "Resolved latest release to ${VERSION}"
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

list_standalone_skills() {
  local skills_source="$1"
  find "${skills_source}" -mindepth 1 -maxdepth 1 -type d \
    ! -name cli \
    ! -name mcp \
    ! -name shared \
    ! -name common \
    -exec test -f "{}/SKILL.md" ';' -print | sed 's#.*/##' | sort
}

install_standalone_skill_from_source() {
  local skills_source="$1"
  local skills_dest="$2"
  local skill_name="$3"
  local skill_source="${skills_source}/${skill_name}"
  local installed_dir="${skills_dest}/${skill_name}"
  local legacy_name=""

  validate_skill_name_segment "${skill_name}"
  if [[ ! -f "${skill_source}/SKILL.md" ]]; then
    echo "Standalone skill '${skill_name}' not found in repository archive." >&2
    return 1
  fi

  mkdir -p "${skills_dest}"
  rm -rf "${installed_dir}"
  cp -R "${skill_source}" "${installed_dir}"

  for legacy_name in "cli-${skill_name}" "mcp-${skill_name}"; do
    if [[ -e "${skills_dest}/${legacy_name}" ]]; then
      rm -rf "${skills_dest:?}/${legacy_name}"
    fi
  done

  echo "Installed standalone skill '${skill_name}' to ${skills_dest}"
}

install_all_standalone_skills_from_source() {
  local skills_source="$1"
  local skills_dest="$2"
  local skill_count=0
  local skill_name=""

  while IFS= read -r skill_name; do
    [[ -n "${skill_name}" ]] || continue
    install_standalone_skill_from_source "${skills_source}" "${skills_dest}" "${skill_name}"
    skill_count=$((skill_count + 1))
  done < <(list_standalone_skills "${skills_source}")

  if (( skill_count < 1 )); then
    return 1
  fi

  echo "Installed ${skill_count} standalone skill(s) to ${skills_dest}"
}

install_legacy_bundle_skills_from_source() {
  local skills_source="$1"
  local skills_dest="$2"
  local skills_bundle="$3"
  local bundle_source="${skills_source}/${skills_bundle}"
  local other_bundle=""
  local other_source=""
  local skill_count=0
  local removed_other_count=0
  local skill_name=""
  local skill_file=""
  local skill_dir=""
  local skill_rel=""
  local installed_dir=""
  local staged_skill_md=""

  if [[ ! -d "${bundle_source}" ]]; then
    echo "Skills bundle '${skills_bundle}' not found in repository archive." >&2
    return 1
  fi
  if [[ ! -d "${skills_source}/shared" ]]; then
    echo "Legacy skill bundle '${skills_bundle}' requires missing shared skill assets." >&2
    return 1
  fi

  if [[ "${skills_bundle}" == "cli" ]]; then
    other_bundle="mcp"
  else
    other_bundle="cli"
  fi
  other_source="${skills_source}/${other_bundle}"

  mkdir -p "${skills_dest}"
  if [[ -d "${other_source}" ]]; then
    while IFS= read -r -d '' skill_file; do
      skill_dir="$(dirname "${skill_file}")"
      skill_rel="${skill_dir#"${other_source}/"}"
      skill_name="${other_bundle}-${skill_rel//\//-}"
      if [[ -e "${skills_dest}/${skill_name}" ]]; then
        rm -rf "${skills_dest:?}/${skill_name}"
        removed_other_count=$((removed_other_count + 1))
      fi
    done < <(find "${other_source}" -type f -name "SKILL.md" -print0)
  fi

  while IFS= read -r -d '' skill_file; do
    skill_dir="$(dirname "${skill_file}")"
    skill_rel="${skill_dir#"${bundle_source}/"}"
    skill_name="${skills_bundle}-${skill_rel//\//-}"
    installed_dir="${skills_dest}/${skill_name}"
    staged_skill_md="${tmp_dir}/${skill_name}-SKILL.md"
    rm -rf "${installed_dir}"
    cp -R "${skill_dir}" "${installed_dir}"
    mkdir -p "${installed_dir}/shared"
    cp -R "${skills_source}/shared/." "${installed_dir}/shared/"
    if [[ -f "${installed_dir}/SKILL.md" ]]; then
      sed 's#\.\./\.\./shared/#shared/#g' "${installed_dir}/SKILL.md" > "${staged_skill_md}"
      mv "${staged_skill_md}" "${installed_dir}/SKILL.md"
    fi
    skill_count=$((skill_count + 1))
  done < <(find "${bundle_source}" -type f -name "SKILL.md" -print0)

  if (( skill_count < 1 )); then
    echo "No skills were found to install for bundle '${skills_bundle}'." >&2
    return 1
  fi

  echo "Installed ${skill_count} ${skills_bundle} skill(s) to ${skills_dest}"
  if (( removed_other_count > 0 )); then
    echo "Removed ${removed_other_count} conflicting ${other_bundle} skill(s) from ${skills_dest}"
  fi
}

install_skills_from_ref() {
  local ref="$1"
  local skills_dest="$2"
  local requested_skill="$3"
  local legacy_bundle="$4"
  local archive_ref="${ref//\//-}"
  local archive_path="${tmp_dir}/repo-${archive_ref}.tar.gz"
  local extract_dir="${tmp_dir}/repo-${archive_ref}"
  local skills_source=""

  download_repo_archive "${ref}" "${archive_path}"
  mkdir -p "${extract_dir}"
  tar -xzf "${archive_path}" -C "${extract_dir}"
  skills_source="$(find "${extract_dir}" -type d -path "*/.github/skills" | head -n 1)"
  if [[ -z "${skills_source}" ]]; then
    echo "Skills directory not found in repository archive for ref '${ref}'." >&2
    return 1
  fi

  if [[ -n "${requested_skill}" ]]; then
    install_standalone_skill_from_source "${skills_source}" "${skills_dest}" "${requested_skill}"
    return $?
  fi

  if install_all_standalone_skills_from_source "${skills_source}" "${skills_dest}"; then
    return 0
  fi

  if [[ -n "${legacy_bundle}" ]]; then
    install_legacy_bundle_skills_from_source "${skills_source}" "${skills_dest}" "${legacy_bundle}"
    return $?
  fi

  echo "No standalone skills found at ref '${ref}'. For legacy refs, pass --skills-bundle <cli|mcp>." >&2
  return 1
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
    --warm-models)
      INSTALL_WARM_MODELS="1"
      ;;
    --install-skills)
      INSTALL_SKILLS="1"
      ;;
    --skills-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --skills-dir" >&2
        exit 1
      fi
      SKILLS_DIR="$2"
      shift
      ;;
    --skills-bundle)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --skills-bundle" >&2
        exit 1
      fi
      SKILLS_BUNDLE="$2"
      shift
      ;;
    --skill)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --skill" >&2
        exit 1
      fi
      SKILL_NAME="$2"
      shift
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

resolve_skill_install_selection

case "${VERIFY_SIGNATURE}" in
  1|auto|0) ;;
  *)
    echo "Invalid DBT_NOVA_VERIFY_SIGNATURE='${VERIFY_SIGNATURE}'. Use 1, auto, or 0." >&2
    exit 1
    ;;
esac

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
    INSTALL_FLAVOR="slim"
  else
    read -r -p "Install bundled artifact with pre-downloaded models? [y/N]: " choice
    choice="${choice:-N}"
    if [[ "${choice,,}" == "y" ]]; then
      INSTALL_FLAVOR="bundled"
    else
      INSTALL_FLAVOR="slim"
    fi
  fi
fi

set_artifact_urls() {
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
  url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${asset}"
  checksum_url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${checksum_file}"
  signature_url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${signature_file}"
  certificate_url="https://github.com/${REPO_SLUG}/releases/download/${VERSION}/${certificate_file}"
}

resolve_latest_version
set_artifact_urls

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

echo "Downloading ${url}"
if ! download_file "${asset}" "${url}" "${tmp_dir}/${asset}"; then
  if [[ "${INSTALL_FLAVOR}" == "bundled" ]]; then
    echo "Bundled artifact unavailable. Falling back to slim artifact." >&2
    INSTALL_FLAVOR="slim"
    set_artifact_urls
    echo "Downloading ${url}"
    download_file "${asset}" "${url}" "${tmp_dir}/${asset}"
  else
    exit 1
  fi
fi

if [[ "${VERIFY_CHECKSUM}" == "1" ]]; then
  echo "Downloading ${checksum_url}"
  download_file "${checksum_file}" "${checksum_url}" "${tmp_dir}/${checksum_file}"
  echo "Verifying SHA-256 checksum"
  verify_checksum_file "${tmp_dir}/${asset}" "${tmp_dir}/${checksum_file}"
fi

if [[ "${VERIFY_SIGNATURE}" != "0" ]]; then
  if [[ "${VERIFY_CHECKSUM}" != "1" ]]; then
    echo "Enabling checksum verification because signature verification requires checksum_file."
    echo "Downloading ${checksum_url}"
    download_file "${checksum_file}" "${checksum_url}" "${tmp_dir}/${checksum_file}"
    VERIFY_CHECKSUM=1
    echo "Verifying SHA-256 checksum"
    verify_checksum_file "${tmp_dir}/${asset}" "${tmp_dir}/${checksum_file}"
  fi

  signature_available=1
  echo "Downloading ${signature_url}"
  if ! download_file "${signature_file}" "${signature_url}" "${tmp_dir}/${signature_file}"; then
    if [[ "${VERIFY_SIGNATURE}" == "auto" ]]; then
      echo "Checksum signature unavailable; skipping automatic signature verification." >&2
      rm -f "${tmp_dir}/${signature_file}"
      signature_available=0
    else
      exit 1
    fi
  fi
  echo "Downloading ${certificate_url}"
  if ! download_file "${certificate_file}" "${certificate_url}" "${tmp_dir}/${certificate_file}"; then
    if [[ "${VERIFY_SIGNATURE}" == "auto" ]]; then
      echo "Checksum certificate unavailable; skipping automatic signature verification." >&2
      rm -f "${tmp_dir}/${certificate_file}"
      signature_available=0
    else
      exit 1
    fi
  fi
  if [[ "${signature_available}" == "1" ]]; then
    verify_signature "${tmp_dir}/${checksum_file}" "${tmp_dir}/${signature_file}" "${tmp_dir}/${certificate_file}"
  fi
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
  echo "Slim install selected. Semantic layers are disabled by default."
  echo "Optional: set DBT_NOVA_INSTALL_WARM_MODELS=1 (or pass --warm-models) to pre-warm models before enabling semantic search."
fi

if [[ "${INSTALL_WARM_MODELS}" == "1" && "${INSTALL_FLAVOR}" == "slim" ]]; then
  warm_script_path="${tmp_dir}/warm_models.sh"
  warm_cache_dir="${DBT_NOVA_EMBEDDINGS_CACHE_DIR:-$HOME/.dbt-nova/.fastembed_cache}"
  warm_required_models="${DBT_NOVA_WARMUP_REQUIRED_MODELS:-3}"
  warm_script_downloaded="0"
  warm_script_url=""

  warm_script_refs=()
  warm_script_refs+=("${VERSION}")

  for warm_script_ref in "${warm_script_refs[@]}"; do
    [[ -n "${warm_script_ref}" ]] || continue
    warm_script_url="https://raw.githubusercontent.com/${REPO_SLUG}/${warm_script_ref}/scripts/warm_models.sh"
    echo "Downloading ${warm_script_url}"
    if download_raw_script "${warm_script_url}" "${warm_script_path}"; then
      warm_script_downloaded="1"
      break
    fi
  done

  if [[ "${warm_script_downloaded}" != "1" ]]; then
    echo "Could not download warm_models.sh; skipping optional model warmup." >&2
  else
    chmod +x "${warm_script_path}"

    echo "Pre-warming models into ${warm_cache_dir} (required snapshots: ${warm_required_models})"
    DBT_NOVA_BIN="${INSTALL_DIR}/dbt-nova${ext}" \
      DBT_NOVA_EMBEDDINGS_CACHE_DIR="${warm_cache_dir}" \
      DBT_NOVA_WARMUP_REQUIRED_MODELS="${warm_required_models}" \
      bash "${warm_script_path}" partial
  fi
fi

if [[ "${INSTALL_SKILLS}" == "1" ]]; then
  skills_refs=()
  skills_installed="0"

  skills_refs+=("${VERSION}")

  for skills_ref in "${skills_refs[@]}"; do
    [[ -n "${skills_ref}" ]] || continue
    if [[ -n "${SKILL_NAME}" ]]; then
      echo "Installing standalone skill '${SKILL_NAME}' from ref '${skills_ref}' into ${SKILLS_DIR}"
    else
      echo "Installing standalone skills from ref '${skills_ref}' into ${SKILLS_DIR}"
    fi
    if install_skills_from_ref "${skills_ref}" "${SKILLS_DIR}" "${SKILL_NAME}" "${SKILLS_BUNDLE}"; then
      skills_installed="1"
      break
    fi
  done

  if [[ "${skills_installed}" != "1" ]]; then
    echo "Failed to install skills into ${SKILLS_DIR}." >&2
    exit 1
  fi
fi

echo "Installed dbt-nova to ${INSTALL_DIR}/dbt-nova${ext}"
echo "Add ${INSTALL_DIR} to your PATH if needed."
