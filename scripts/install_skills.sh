#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SKILLS_SOURCE="${DBT_NOVA_SKILLS_SOURCE:-${REPO_ROOT}/.github/skills}"
SKILLS_DIR="${DBT_NOVA_SKILLS_DIR:-$HOME/.agents/skills}"
SKILLS_BUNDLE="${DBT_NOVA_SKILLS_BUNDLE:-}"

usage() {
  cat <<'EOF'
Usage: install_skills.sh --bundle <cli|mcp> [--skills-dir <path>] [--skills-source <path>]

Installs one dbt-nova skill bundle from the current checkout into a destination
skills directory. The installed skills are standalone: shared references/assets
are copied into each skill directory, and any previously installed dbt-nova
skills from the other bundle are removed from the same destination.

Environment overrides:
  DBT_NOVA_SKILLS_BUNDLE    cli|mcp bundle to install
  DBT_NOVA_SKILLS_DIR       Destination skills directory (default: $HOME/.agents/skills)
  DBT_NOVA_SKILLS_SOURCE    Source .github/skills directory (default: repo checkout)
EOF
}

validate_bundle() {
  case "$1" in
    cli|mcp) ;;
    *)
      echo "Invalid skills bundle '$1'. Use 'cli' or 'mcp'." >&2
      return 1
      ;;
  esac
}

install_from_source() {
  local source_root="$1"
  local skills_dest="$2"
  local skills_bundle="$3"
  local bundle_source="${source_root}/${skills_bundle}"
  local other_bundle=""
  local other_source=""
  local skill_file=""
  local skill_dir=""
  local skill_rel=""
  local skill_name=""
  local installed_dir=""
  local staged_skill_md=""
  local removed_other_count=0
  local installed_count=0
  local tmp_dir

  if [[ ! -d "${source_root}" ]]; then
    echo "Skills source directory not found: ${source_root}" >&2
    return 1
  fi
  if [[ ! -d "${bundle_source}" ]]; then
    echo "Skills bundle '${skills_bundle}' not found in ${source_root}." >&2
    return 1
  fi

  if [[ "${skills_bundle}" == "cli" ]]; then
    other_bundle="mcp"
  else
    other_bundle="cli"
  fi
  other_source="${source_root}/${other_bundle}"

  tmp_dir="$(mktemp -d)"

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
    cp -R "${source_root}/shared/." "${installed_dir}/shared/"
    sed 's#\.\./\.\./shared/#shared/#g' "${installed_dir}/SKILL.md" > "${staged_skill_md}"
    mv "${staged_skill_md}" "${installed_dir}/SKILL.md"

    installed_count=$((installed_count + 1))
  done < <(find "${bundle_source}" -type f -name "SKILL.md" -print0)

  if (( installed_count < 1 )); then
    rm -rf "${tmp_dir}"
    echo "No skills were found to install for bundle '${skills_bundle}'." >&2
    return 1
  fi

  echo "Installed ${installed_count} ${skills_bundle} skill(s) to ${skills_dest}"
  if (( removed_other_count > 0 )); then
    echo "Removed ${removed_other_count} conflicting ${other_bundle} skill(s) from ${skills_dest}"
  fi
  rm -rf "${tmp_dir}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --bundle" >&2
        exit 1
      fi
      SKILLS_BUNDLE="$2"
      shift
      ;;
    --skills-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --skills-dir" >&2
        exit 1
      fi
      SKILLS_DIR="$2"
      shift
      ;;
    --skills-source)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --skills-source" >&2
        exit 1
      fi
      SKILLS_SOURCE="$2"
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

if [[ -z "${SKILLS_BUNDLE}" ]]; then
  echo "Installing skills requires --bundle <cli|mcp> or DBT_NOVA_SKILLS_BUNDLE." >&2
  exit 1
fi

validate_bundle "${SKILLS_BUNDLE}"
install_from_source "${SKILLS_SOURCE}" "${SKILLS_DIR}" "${SKILLS_BUNDLE}"
