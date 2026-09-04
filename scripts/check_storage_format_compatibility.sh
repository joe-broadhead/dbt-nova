#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 <current-binary> <legacy-binary> <manifest.json>" >&2
  exit 2
fi

current_binary="$1"
legacy_binary="$2"
manifest_path="$3"

for binary in "${current_binary}" "${legacy_binary}"; do
  if [[ ! -x "${binary}" ]]; then
    echo "binary is not executable: ${binary}" >&2
    exit 2
  fi
done
if [[ ! -f "${manifest_path}" ]]; then
  echo "manifest does not exist: ${manifest_path}" >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 2
fi

workspace="$(mktemp -d)"
trap 'rm -rf "${workspace}"' EXIT

storage_dir="${workspace}/storage"
instance_id="storage-format-compat"
legacy_load="${workspace}/legacy-load.json"
current_legacy_read="${workspace}/current-legacy-read.json"
current_remote_hydration="${workspace}/current-remote-hydration.json"
current_load="${workspace}/current-load.json"
legacy_rollback="${workspace}/legacy-rollback.json"
legacy_contract_rejection="${workspace}/legacy-contract-rejection.json"

run_load() {
  local binary="$1"
  local output="$2"
  shift 2
  DBT_NOVA_SEARCH_ENABLE_VECTOR=false \
    DBT_NOVA_SEARCH_ENABLE_SPARSE=false \
    DBT_NOVA_SEARCH_ENABLE_RERANKER=false \
    DBT_NOVA_STORAGE_DIR="${storage_dir}" \
    "${binary}" manifest load \
      --manifest-path "${manifest_path}" \
      --storage-instance-id "${instance_id}" \
      --json \
      "$@" > "${output}"
  jq -e '.status == "success"' "${output}" >/dev/null
}

run_load "${legacy_binary}" "${legacy_load}"
legacy_version="$(jq -r '.data.manifest_version' "${legacy_load}")"

run_load "${current_binary}" "${current_legacy_read}" --read-only
jq -e --arg version "${legacy_version}" '
  .data.manifest_version == $version and
  .data.storage_format_version == "nova-storage-v1" and
  .data.reused.entity_store == true and
  .data.reused.tantivy == true and
  .data.reused.indexes == true
' "${current_legacy_read}" >/dev/null

# Consumer-first rollout must also work on a cold host. A current writable
# consumer validates the v1-scoped hash, hydrates the old archive, and rebuilds
# it into the current format without mutating the retained v1 generation.
legacy_manifest_hash="$(jq -r '.data.manifest_hash' "${legacy_load}")"
legacy_storage_archive="${workspace}/legacy-storage.tar.gz"
legacy_metadata_path="${workspace}/legacy-metadata.json"
COPYFILE_DISABLE=1 tar --exclude='*.lock' -czf "${legacy_storage_archive}" \
  -C "$(dirname "${storage_dir}")" "$(basename "${storage_dir}")"
jq -n \
  --arg manifest_hash "${legacy_manifest_hash}" \
  --arg manifest_version "${legacy_version}" \
  '{
    contract_version: "v1",
    manifest_hash: $manifest_hash,
    manifest_version: $manifest_version,
    entity_count: 0,
    storage_instance_id: "storage-format-compat",
    dbt_nova_version: "0.0.6",
    build_timestamp: "2026-01-01T00:00:00Z",
    artifact_name_storage: "legacy-storage",
    artifact_name_models: ""
  }' > "${legacy_metadata_path}"

remote_storage_dir="${workspace}/remote-storage"
DBT_NOVA_SEARCH_ENABLE_VECTOR=false \
  DBT_NOVA_SEARCH_ENABLE_SPARSE=false \
  DBT_NOVA_SEARCH_ENABLE_RERANKER=false \
  DBT_NOVA_STORAGE_DIR="${remote_storage_dir}" \
  DBT_NOVA_STORAGE_ARTIFACT_URI="file://${legacy_storage_archive}" \
  DBT_NOVA_METADATA_ARTIFACT_URI="file://${legacy_metadata_path}" \
  DBT_NOVA_ARTIFACT_FETCH_POLICY=always \
  "${current_binary}" manifest load \
    --manifest-path "${manifest_path}" \
    --storage-instance-id "${instance_id}" \
    --json > "${current_remote_hydration}"
jq -e '
  .status == "success" and
  .data.storage_format_version == "nova-storage-v2" and
  .data.reused.entity_store == false and
  .data.reused.tantivy == false and
  .data.reused.indexes == false
' "${current_remote_hydration}" >/dev/null

run_load "${current_binary}" "${current_load}"
current_version="$(jq -r '.data.manifest_version' "${current_load}")"
current_format="$(jq -r '.data.storage_format_version // empty' "${current_load}")"

if [[ -z "${legacy_version}" || -z "${current_version}" ]]; then
  echo "manifest load did not return version identifiers" >&2
  exit 1
fi
if [[ "${legacy_version}" == "${current_version}" ]]; then
  echo "storage format change did not produce a distinct manifest version" >&2
  exit 1
fi
if [[ "${current_format}" != "nova-storage-v2" ]]; then
  echo "unexpected current storage format: ${current_format}" >&2
  exit 1
fi
jq -e '
  .data.reused.entity_store == false and
  .data.reused.tantivy == false and
  .data.reused.indexes == false
' "${current_load}" >/dev/null

# The legacy binary must recover its retained format-specific version even
# after the current binary advances manifest.current.json.
run_load "${legacy_binary}" "${legacy_rollback}" --read-only
jq -e --arg version "${legacy_version}" '
  .data.manifest_version == $version and
  .data.reused.entity_store == true and
  .data.reused.tantivy == true and
  .data.reused.indexes == true
' "${legacy_rollback}" >/dev/null

# A cold legacy consumer cannot read the new Tantivy format. The v2 metadata
# contract must stop it before archive extraction with an explicit upgrade cue.
metadata_path="${workspace}/nova-build-metadata.json"
storage_archive="${workspace}/storage.tar.gz"
printf 'not-read-before-contract-validation' > "${storage_archive}"
jq -n \
  --arg manifest_hash "${current_version}" \
  '{
    contract_version: "v2",
    storage_format_version: "nova-storage-v2",
    manifest_hash: $manifest_hash,
    manifest_version: $manifest_hash,
    entity_count: 0,
    storage_instance_id: "storage-format-compat",
    dbt_nova_version: "0.0.6",
    build_timestamp: "2026-01-01T00:00:00Z",
    artifact_name_storage: "storage-format-compat",
    artifact_name_models: ""
  }' > "${metadata_path}"

set +e
DBT_NOVA_SEARCH_ENABLE_VECTOR=false \
  DBT_NOVA_SEARCH_ENABLE_SPARSE=false \
  DBT_NOVA_SEARCH_ENABLE_RERANKER=false \
  DBT_NOVA_STORAGE_DIR="${workspace}/legacy-cold-storage" \
  DBT_NOVA_STORAGE_ARTIFACT_URI="file://${storage_archive}" \
  DBT_NOVA_METADATA_ARTIFACT_URI="file://${metadata_path}" \
  DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing \
  "${legacy_binary}" manifest load \
    --manifest-path "${manifest_path}" \
    --storage-instance-id "${instance_id}" \
    --json > "${legacy_contract_rejection}" 2>/dev/null
legacy_contract_status="$?"
set -e

if [[ "${legacy_contract_status}" -eq 0 ]]; then
  echo "legacy consumer unexpectedly accepted the v2 prebuilt contract" >&2
  exit 1
fi
jq -e '
  .status == "error" and
  (.error.error | contains("unsupported prebuilt metadata contract_version"))
' "${legacy_contract_rejection}" >/dev/null

echo "storage compatibility passed: legacy=${legacy_version} current=${current_version} format=${current_format}"
