#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SKILLS_SOURCE="${DBT_NOVA_SKILLS_SOURCE:-${REPO_ROOT}/.github/skills}"
SKILLS_DIR="${DBT_NOVA_SKILLS_DIR:-$HOME/.agents/skills}"
SKILL_NAME="${DBT_NOVA_SKILL_NAME:-}"
INSTALL_ALL="${DBT_NOVA_INSTALL_ALL:-}"

usage() {
  cat <<'EOF'
Usage:
  install_skills.sh --skill <name> [--skills-dir <path>] [--skills-source <path>]
  install_skills.sh --all [--skills-dir <path>] [--skills-source <path>]

Deprecated:
  install_skills.sh --bundle <cli|mcp> [...]

Installs one standalone persona-first skill, or all standalone skills, from the
current checkout into a destination skills directory.

Environment overrides:
  DBT_NOVA_SKILL_NAME       standalone skill to install (for example: analyst)
  DBT_NOVA_INSTALL_ALL      set to 1 to install all standalone skills
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

list_installable_skills() {
  local source_root="$1"
  find "${source_root}" -mindepth 1 -maxdepth 1 -type d \
    ! -name cli \
    ! -name mcp \
    ! -name shared \
    ! -name common \
    -exec test -f "{}/SKILL.md" ';' -print | sed 's#.*/##' | sort
}

validate_skill_name() {
  local source_root="$1"
  local skill_name="$2"
  validate_skill_name_segment "${skill_name}"
  if [[ ! -d "${source_root}/${skill_name}" ]]; then
    echo "Standalone skill '${skill_name}' not found in ${source_root}." >&2
    return 1
  fi
  if [[ "${skill_name}" == "cli" || "${skill_name}" == "mcp" || "${skill_name}" == "shared" || "${skill_name}" == "common" ]]; then
    echo "'${skill_name}' is not an installable standalone skill." >&2
    return 1
  fi
  if [[ ! -f "${source_root}/${skill_name}/SKILL.md" ]]; then
    echo "Standalone skill '${skill_name}' is missing SKILL.md." >&2
    return 1
  fi
}

install_standalone_skill() {
  local source_root="$1"
  local skills_dest="$2"
  local skill_name="$3"
  local skill_source="${source_root}/${skill_name}"
  local installed_dir="${skills_dest}/${skill_name}"
  local legacy_name=""

  validate_skill_name "${source_root}" "${skill_name}"

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

install_all_skills() {
  local source_root="$1"
  local skills_dest="$2"
  local skill_name=""
  local installed_count=0

  while IFS= read -r skill_name; do
    [[ -z "${skill_name}" ]] && continue
    install_standalone_skill "${source_root}" "${skills_dest}" "${skill_name}"
    installed_count=$((installed_count + 1))
  done < <(list_installable_skills "${source_root}")

  if (( installed_count < 1 )); then
    echo "No standalone skills were found in ${source_root}." >&2
    return 1
  fi

  echo "Installed ${installed_count} standalone skill(s) to ${skills_dest}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --bundle" >&2
        exit 1
      fi
      validate_bundle "$2"
      INSTALL_ALL=1
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
    --all)
      INSTALL_ALL=1
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

if [[ -n "${SKILL_NAME}" && -n "${INSTALL_ALL}" ]]; then
  echo "Use either --skill or --all, not both." >&2
  exit 1
fi

if [[ -n "${INSTALL_ALL}" ]]; then
  install_all_skills "${SKILLS_SOURCE}" "${SKILLS_DIR}"
  exit 0
fi

if [[ -n "${SKILL_NAME}" ]]; then
  install_standalone_skill "${SKILLS_SOURCE}" "${SKILLS_DIR}" "${SKILL_NAME}"
  exit 0
fi

echo "Installing skills requires either --skill <name> or --all." >&2
exit 1
