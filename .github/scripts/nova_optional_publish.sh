#!/usr/bin/env bash
set -euo pipefail

publish_targets="${DBT_NOVA_PUBLISH_TARGETS:-}"
publish_dry_run="${DBT_NOVA_PUBLISH_DRY_RUN:-false}"
storage_archive="${STORAGE_ARCHIVE_NAME}"
manifest_source="${MANIFEST_SOURCE_PATH}"
manifest_filename="${ARTIFACT_NAME_MANIFEST}.json"
manifest_publish_path="out/${manifest_filename}"
metadata_source="${METADATA_SOURCE_PATH}"
metadata_filename="${ARTIFACT_NAME_METADATA}.json"
metadata_publish_path="out/${metadata_filename}"
bootstrap_filename="${ARTIFACT_NAME_BOOTSTRAP}.json"
bootstrap_latest_filename="${DBT_NOVA_STORAGE_INSTANCE_ID}-latest-bootstrap.json"
publish_summary_filename="${ARTIFACT_NAME_PUBLISH_SUMMARY}.json"
publish_summary_path="out/${publish_summary_filename}"
bootstrap_dir="out/bootstrap"
mkdir -p "${bootstrap_dir}"
cp "${manifest_source}" "${manifest_publish_path}"
cp "${metadata_source}" "${metadata_publish_path}"

models_archive=""
if [[ "${INPUT_PUBLISH_MODELS_ARCHIVE}" == "true" ]]; then
  models_archive="${ARTIFACT_NAME_MODELS}.tar.gz"
fi
include_models_in_bootstrap="${INPUT_INCLUDE_MODELS_IN_BOOTSTRAP}"

if [[ -z "${publish_targets}" ]]; then
  jq -n \
    --arg published_targets "" \
    --argjson published_storage_uris '{}' \
    --argjson published_manifest_uris '{}' \
    --argjson published_metadata_uris '{}' \
    --argjson published_bootstrap_uris '{}' \
    --argjson published_bootstrap_latest_uris '{}' \
    --argjson published_models_uris '{}' \
    '{
      published_targets: $published_targets,
      published_storage_uris: $published_storage_uris,
      published_manifest_uris: $published_manifest_uris,
      published_metadata_uris: $published_metadata_uris,
      published_bootstrap_uris: $published_bootstrap_uris,
      published_bootstrap_latest_uris: $published_bootstrap_latest_uris,
      published_models_uris: $published_models_uris
    }' > "${publish_summary_path}"
  {
    echo "published_targets="
    echo "publish_summary_path=${publish_summary_path}"
    echo 'published_bootstrap_latest_uris={}'
    echo 'published_storage_uris_legacy={}'
    echo 'published_metadata_uris_legacy={}'
    echo 'published_manifest_uris_legacy={}'
    echo 'published_bootstrap_uris_legacy={}'
    echo 'published_models_uris_legacy={}'
  } >> "$GITHUB_OUTPUT"
  {
    echo "### Remote Publish"
    echo ""
    echo "Remote publish disabled (publish_targets is empty)."
    echo "- publish_summary_path: \`${publish_summary_path}\`"
  } >> "$GITHUB_STEP_SUMMARY"
  exit 0
fi

retry_with_backoff() {
  local max_attempts="$1"
  shift
  local attempt=1
  local delay_secs=2
  while true; do
    if "$@"; then
      return 0
    fi
    if (( attempt >= max_attempts )); then
      return 1
    fi
    sleep "${delay_secs}"
    delay_secs=$((delay_secs * 2))
    attempt=$((attempt + 1))
  done
}

publish_to_s3() {
  local local_path="$1"
  local remote_uri="$2"
  aws s3 cp --only-show-errors "${local_path}" "${remote_uri}"
}

download_from_s3() {
  local remote_uri="$1"
  local local_path="$2"
  local bucket object stderr_path

  if [[ "${remote_uri}" != s3://* ]]; then
    echo "invalid S3 URI: ${remote_uri}"
    return 1
  fi
  bucket="${remote_uri#s3://}"
  bucket="${bucket%%/*}"
  object="${remote_uri#s3://${bucket}/}"
  if [[ -z "${bucket}" || -z "${object}" || "${object}" == "${remote_uri}" ]]; then
    echo "invalid S3 URI: ${remote_uri}"
    return 1
  fi

  stderr_path="$(mktemp)"
  if aws s3api get-object --bucket "${bucket}" --key "${object}" "${local_path}" >/dev/null 2>"${stderr_path}"; then
    rm -f "${stderr_path}"
    return 0
  fi

  if grep -Eqi '(NoSuchKey|Not Found|404)' "${stderr_path}"; then
    rm -f "${stderr_path}" "${local_path}"
    return 10
  fi

  cat "${stderr_path}" >&2
  rm -f "${stderr_path}" "${local_path}"
  return 1
}

publish_to_gcs() {
  local local_path="$1"
  local remote_uri="$2"
  local bucket object object_encoded

  if [[ "${remote_uri}" != gs://* ]]; then
    echo "invalid GCS URI: ${remote_uri}"
    return 1
  fi
  bucket="${remote_uri#gs://}"
  bucket="${bucket%%/*}"
  object="${remote_uri#gs://${bucket}/}"
  if [[ -z "${bucket}" || -z "${object}" || "${object}" == "${remote_uri}" ]]; then
    echo "invalid GCS URI: ${remote_uri}"
    return 1
  fi

  object_encoded="$(python -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "${object}")"

  curl -fsS \
    -X POST \
    -H "Authorization: Bearer ${GCS_ACCESS_TOKEN}" \
    -H "Content-Type: application/octet-stream" \
    --data-binary "@${local_path}" \
    "https://storage.googleapis.com/upload/storage/v1/b/${bucket}/o?uploadType=media&name=${object_encoded}" \
    >/dev/null
}

download_from_gcs() {
  local remote_uri="$1"
  local local_path="$2"
  local bucket object object_encoded http_code

  if [[ "${remote_uri}" != gs://* ]]; then
    echo "invalid GCS URI: ${remote_uri}"
    return 1
  fi
  bucket="${remote_uri#gs://}"
  bucket="${bucket%%/*}"
  object="${remote_uri#gs://${bucket}/}"
  if [[ -z "${bucket}" || -z "${object}" || "${object}" == "${remote_uri}" ]]; then
    echo "invalid GCS URI: ${remote_uri}"
    return 1
  fi

  object_encoded="$(python -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "${object}")"

  http_code="$(curl -sS \
    -H "Authorization: Bearer ${GCS_ACCESS_TOKEN}" \
    -o "${local_path}" \
    -w "%{http_code}" \
    "https://storage.googleapis.com/storage/v1/b/${bucket}/o/${object_encoded}?alt=media")" || {
    rm -f "${local_path}"
    return 1
  }

  case "${http_code}" in
    200)
      return 0
      ;;
    404)
      rm -f "${local_path}"
      return 10
      ;;
    *)
      echo "unexpected GCS read status ${http_code} for ${remote_uri}" >&2
      rm -f "${local_path}"
      return 1
      ;;
  esac
}

publish_to_dbfs() {
  local local_path="$1"
  local remote_path="$2"
  python -c 'import sys, textwrap; exec(textwrap.dedent(sys.stdin.read()))' "${local_path}" "${remote_path}" <<'PY'
import base64
import json
import os
import sys
import urllib.request

local_path = sys.argv[1]
remote_uri = sys.argv[2]
host = os.environ["DATABRICKS_HOST"].rstrip("/")
token = os.environ["DATABRICKS_ACCESS_TOKEN"]
headers = {
    "Authorization": f"Bearer {token}",
    "Content-Type": "application/json",
}

if remote_uri.startswith("dbfs:/"):
    remote_path = "/" + remote_uri[len("dbfs:/"):].lstrip("/")
elif remote_uri.startswith("/"):
    remote_path = remote_uri
else:
    raise ValueError(f"Invalid DBFS path: {remote_uri}")

def call_dbfs(endpoint: str, payload: dict) -> dict:
    request = urllib.request.Request(
        f"{host}/api/2.0/dbfs/{endpoint}",
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        body = response.read()
    if not body:
        return {}
    return json.loads(body.decode("utf-8"))

handle = call_dbfs("create", {"path": remote_path, "overwrite": True})["handle"]
chunk_size = 1024 * 768
with open(local_path, "rb") as file_handle:
    while True:
        chunk = file_handle.read(chunk_size)
        if not chunk:
            break
        call_dbfs(
            "add-block",
            {
                "handle": handle,
                "data": base64.b64encode(chunk).decode("utf-8"),
            },
        )
call_dbfs("close", {"handle": handle})
PY
}

download_from_dbfs() {
  local remote_path="$1"
  local local_path="$2"
  python -c 'import sys, textwrap; exec(textwrap.dedent(sys.stdin.read()))' "${remote_path}" "${local_path}" <<'PY'
import base64
import json
import os
import sys
import urllib.error
import urllib.request

remote_uri = sys.argv[1]
local_path = sys.argv[2]
host = os.environ["DATABRICKS_HOST"].rstrip("/")
token = os.environ["DATABRICKS_ACCESS_TOKEN"]
headers = {
    "Authorization": f"Bearer {token}",
    "Content-Type": "application/json",
}

if remote_uri.startswith("dbfs:/"):
    remote_path = "/" + remote_uri[len("dbfs:/"):].lstrip("/")
elif remote_uri.startswith("/"):
    remote_path = remote_uri
else:
    raise ValueError(f"Invalid DBFS path: {remote_uri}")

def call_dbfs(endpoint: str, payload: dict) -> dict:
    request = urllib.request.Request(
        f"{host}/api/2.0/dbfs/{endpoint}",
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        body = response.read()
    if not body:
        return {}
    return json.loads(body.decode("utf-8"))

try:
    call_dbfs("get-status", {"path": remote_path})
except urllib.error.HTTPError as err:
    if err.code == 404:
        sys.exit(10)
    raise

offset = 0
chunk_size = 1024 * 1024
with open(local_path, "wb") as file_handle:
    while True:
        payload = call_dbfs(
            "read",
            {"path": remote_path, "offset": offset, "length": chunk_size},
        )
        data = payload.get("data", "")
        bytes_read = int(payload.get("bytes_read", 0))
        if data:
            file_handle.write(base64.b64decode(data))
        if bytes_read <= 0:
            break
        offset += bytes_read
        if bytes_read < chunk_size:
            break
PY
}

should_publish_bootstrap_alias() {
  local candidate_path="$1"
  local existing_path="$2"
  python -c 'import sys, textwrap; exec(textwrap.dedent(sys.stdin.read()))' "${candidate_path}" "${existing_path}" <<'PY'
import json
import sys
from pathlib import Path

candidate_path = Path(sys.argv[1])
existing_path = Path(sys.argv[2])

with candidate_path.open("r", encoding="utf-8") as handle:
    candidate = json.load(handle)

try:
    with existing_path.open("r", encoding="utf-8") as handle:
        existing = json.load(handle)
except Exception:
    sys.exit(0)

candidate_ts = str(candidate.get("build_timestamp", "")).strip()
existing_ts = str(existing.get("build_timestamp", "")).strip()
candidate_hash = str(candidate.get("manifest_hash", "")).strip()
existing_hash = str(existing.get("manifest_hash", "")).strip()

if not existing_ts:
    sys.exit(0)
if candidate_ts > existing_ts:
    sys.exit(0)
if candidate_ts < existing_ts:
    sys.exit(1)
if candidate_hash == existing_hash:
    sys.exit(1)
sys.exit(1)
PY
}

publish_bootstrap_alias_if_newer() {
  local target="$1"
  local local_path="$2"
  local remote_uri="$3"
  local existing_path="$4"
  local download_rc=0

  rm -f "${existing_path}"
  case "${target}" in
    s3)
      if download_from_s3 "${remote_uri}" "${existing_path}"; then
        download_rc=0
      else
        download_rc=$?
      fi
      ;;
    gcs)
      if download_from_gcs "${remote_uri}" "${existing_path}"; then
        download_rc=0
      else
        download_rc=$?
      fi
      ;;
    dbfs)
      if download_from_dbfs "${remote_uri}" "${existing_path}"; then
        download_rc=0
      else
        download_rc=$?
      fi
      ;;
    *)
      echo "unsupported publish target for bootstrap alias: ${target}"
      return 1
      ;;
  esac

  case "${download_rc}" in
    0|10)
      ;;
    *)
      echo "failed to read current stable bootstrap alias for ${target}: ${remote_uri}" >&2
      return "${download_rc}"
      ;;
  esac

  if ! should_publish_bootstrap_alias "${local_path}" "${existing_path}"; then
    echo "skipping stable bootstrap alias update for ${target}; existing alias is newer or equivalent"
    return 0
  fi

  case "${target}" in
    s3)
      publish_to_s3 "${local_path}" "${remote_uri}"
      ;;
    gcs)
      publish_to_gcs "${local_path}" "${remote_uri}"
      ;;
    dbfs)
      publish_to_dbfs "${local_path}" "${remote_uri}"
      ;;
  esac
}

resolve_gcs_access_token() {
  local token=""
  for var_name in \
    DBT_NOVA_GCP_ACCESS_TOKEN \
    DBT_NOVA_BIGQUERY_ACCESS_TOKEN \
    GCP_ACCESS_TOKEN \
    GOOGLE_OAUTH_ACCESS_TOKEN
  do
    token="${!var_name:-}"
    if [[ -n "${token}" ]]; then
      printf '%s' "${token}"
      return 0
    fi
  done

  if command -v gcloud >/dev/null 2>&1; then
    token="$(gcloud auth application-default print-access-token 2>/dev/null || true)"
    if [[ -n "${token}" ]]; then
      printf '%s' "${token}"
      return 0
    fi
  fi
  return 1
}

manifest_hash="$(jq -r '.manifest_hash // empty' "${metadata_source}")"
dbt_nova_version="$(jq -r '.dbt_nova_version // empty' "${metadata_source}")"
build_timestamp="$(jq -r '.build_timestamp // empty' "${metadata_source}")"
if [[ -z "${manifest_hash}" || -z "${dbt_nova_version}" || -z "${build_timestamp}" ]]; then
  echo "metadata contract is missing manifest_hash/dbt_nova_version/build_timestamp"
  exit 1
fi

write_bootstrap_contract() {
  local output_path="$1"
  local profile="$2"
  local manifest_uri_value="$3"
  local storage_uri_value="$4"
  local metadata_uri_value="$5"
  local models_uri_value="$6"
  jq -n \
    --arg contract_version "v1" \
    --arg profile "${profile}" \
    --arg storage_instance_id "${DBT_NOVA_STORAGE_INSTANCE_ID}" \
    --arg manifest_uri "${manifest_uri_value}" \
    --arg storage_artifact_uri "${storage_uri_value}" \
    --arg metadata_artifact_uri "${metadata_uri_value}" \
    --arg models_artifact_uri "${models_uri_value}" \
    --arg manifest_hash "${manifest_hash}" \
    --arg dbt_nova_version "${dbt_nova_version}" \
    --arg build_timestamp "${build_timestamp}" \
    '{
      contract_version: $contract_version,
      profile: $profile,
      storage_instance_id: $storage_instance_id,
      manifest_uri: $manifest_uri,
      storage_artifact_uri: $storage_artifact_uri,
      metadata_artifact_uri: $metadata_artifact_uri,
      models_artifact_uri: $models_artifact_uri,
      manifest_hash: $manifest_hash,
      dbt_nova_version: $dbt_nova_version,
      build_timestamp: $build_timestamp
    }' > "${output_path}"
}

s3_storage_uri=""
s3_manifest_uri=""
s3_metadata_uri=""
s3_bootstrap_uri=""
s3_bootstrap_latest_uri=""
s3_models_uri=""
gcs_storage_uri=""
gcs_manifest_uri=""
gcs_metadata_uri=""
gcs_bootstrap_uri=""
gcs_bootstrap_latest_uri=""
gcs_models_uri=""
dbfs_storage_uri=""
dbfs_manifest_uri=""
dbfs_metadata_uri=""
dbfs_bootstrap_uri=""
dbfs_bootstrap_latest_uri=""
dbfs_models_uri=""
published_targets_success=""

IFS=',' read -r -a publish_targets_array <<< "${publish_targets}"
for target in "${publish_targets_array[@]}"; do
  case "${target}" in
    s3)
      s3_storage_uri="${DBT_NOVA_PUBLISH_S3_PREFIX}/${storage_archive}"
      s3_manifest_uri="${DBT_NOVA_PUBLISH_S3_PREFIX}/${manifest_filename}"
      s3_metadata_uri="${DBT_NOVA_PUBLISH_S3_PREFIX}/${metadata_filename}"
      s3_bootstrap_uri="${DBT_NOVA_PUBLISH_S3_PREFIX}/${bootstrap_filename}"
      s3_bootstrap_latest_uri="${DBT_NOVA_PUBLISH_S3_PREFIX}/${bootstrap_latest_filename}"
      if [[ -n "${models_archive}" ]]; then
        s3_models_uri="${DBT_NOVA_PUBLISH_S3_PREFIX}/${models_archive}"
      fi
      s3_bootstrap_models_uri=""
      if [[ "${include_models_in_bootstrap}" == "true" ]]; then
        s3_bootstrap_models_uri="${s3_models_uri}"
      fi
      s3_bootstrap_path="${bootstrap_dir}/s3-${bootstrap_filename}"
      s3_bootstrap_latest_path="${bootstrap_dir}/s3-${bootstrap_latest_filename}"
      s3_bootstrap_existing_path="${bootstrap_dir}/s3-existing-${bootstrap_latest_filename}"
      write_bootstrap_contract \
        "${s3_bootstrap_path}" \
        "s3" \
        "${s3_manifest_uri}" \
        "${s3_storage_uri}" \
        "${s3_metadata_uri}" \
        "${s3_bootstrap_models_uri}"
      cp "${s3_bootstrap_path}" "${s3_bootstrap_latest_path}"
      if [[ "${publish_dry_run}" != "true" ]]; then
        if ! command -v aws >/dev/null 2>&1; then
          echo "publish target s3 requires aws CLI."
          exit 1
        fi
        retry_with_backoff 4 publish_to_s3 "${storage_archive}" "${s3_storage_uri}" || {
          echo "failed uploading storage archive to s3 after retries"
          exit 1
        }
        retry_with_backoff 4 publish_to_s3 "${manifest_publish_path}" "${s3_manifest_uri}" || {
          echo "failed uploading manifest to s3 after retries"
          exit 1
        }
        retry_with_backoff 4 publish_to_s3 "${metadata_publish_path}" "${s3_metadata_uri}" || {
          echo "failed uploading metadata contract to s3 after retries"
          exit 1
        }
        if [[ -n "${models_archive}" ]]; then
          retry_with_backoff 4 publish_to_s3 "${models_archive}" "${s3_models_uri}" || {
            echo "failed uploading models archive to s3 after retries"
            exit 1
          }
        fi
        retry_with_backoff 4 publish_to_s3 "${s3_bootstrap_path}" "${s3_bootstrap_uri}" || {
          echo "failed uploading bootstrap contract to s3 after retries"
          exit 1
        }
        retry_with_backoff 4 publish_bootstrap_alias_if_newer s3 "${s3_bootstrap_latest_path}" "${s3_bootstrap_latest_uri}" "${s3_bootstrap_existing_path}" || {
          echo "failed uploading stable bootstrap alias to s3 after retries"
          exit 1
        }
      fi
      ;;
    gcs)
      gcs_storage_uri="${DBT_NOVA_PUBLISH_GCS_PREFIX}/${storage_archive}"
      gcs_manifest_uri="${DBT_NOVA_PUBLISH_GCS_PREFIX}/${manifest_filename}"
      gcs_metadata_uri="${DBT_NOVA_PUBLISH_GCS_PREFIX}/${metadata_filename}"
      gcs_bootstrap_uri="${DBT_NOVA_PUBLISH_GCS_PREFIX}/${bootstrap_filename}"
      gcs_bootstrap_latest_uri="${DBT_NOVA_PUBLISH_GCS_PREFIX}/${bootstrap_latest_filename}"
      if [[ -n "${models_archive}" ]]; then
        gcs_models_uri="${DBT_NOVA_PUBLISH_GCS_PREFIX}/${models_archive}"
      fi
      gcs_bootstrap_models_uri=""
      if [[ "${include_models_in_bootstrap}" == "true" ]]; then
        gcs_bootstrap_models_uri="${gcs_models_uri}"
      fi
      gcs_bootstrap_path="${bootstrap_dir}/gcs-${bootstrap_filename}"
      gcs_bootstrap_latest_path="${bootstrap_dir}/gcs-${bootstrap_latest_filename}"
      gcs_bootstrap_existing_path="${bootstrap_dir}/gcs-existing-${bootstrap_latest_filename}"
      write_bootstrap_contract \
        "${gcs_bootstrap_path}" \
        "gcs" \
        "${gcs_manifest_uri}" \
        "${gcs_storage_uri}" \
        "${gcs_metadata_uri}" \
        "${gcs_bootstrap_models_uri}"
      cp "${gcs_bootstrap_path}" "${gcs_bootstrap_latest_path}"
      if [[ "${publish_dry_run}" != "true" ]]; then
        GCS_ACCESS_TOKEN="$(resolve_gcs_access_token)" || {
          echo "publish target gcs requires a Google access token (DBT_NOVA_GCP_ACCESS_TOKEN, DBT_NOVA_BIGQUERY_ACCESS_TOKEN, GCP_ACCESS_TOKEN, GOOGLE_OAUTH_ACCESS_TOKEN, or gcloud ADC)."
          exit 1
        }
        retry_with_backoff 4 publish_to_gcs "${storage_archive}" "${gcs_storage_uri}" || {
          echo "failed uploading storage archive to gcs after retries"
          exit 1
        }
        retry_with_backoff 4 publish_to_gcs "${manifest_publish_path}" "${gcs_manifest_uri}" || {
          echo "failed uploading manifest to gcs after retries"
          exit 1
        }
        retry_with_backoff 4 publish_to_gcs "${metadata_publish_path}" "${gcs_metadata_uri}" || {
          echo "failed uploading metadata contract to gcs after retries"
          exit 1
        }
        if [[ -n "${models_archive}" ]]; then
          retry_with_backoff 4 publish_to_gcs "${models_archive}" "${gcs_models_uri}" || {
            echo "failed uploading models archive to gcs after retries"
            exit 1
          }
        fi
        retry_with_backoff 4 publish_to_gcs "${gcs_bootstrap_path}" "${gcs_bootstrap_uri}" || {
          echo "failed uploading bootstrap contract to gcs after retries"
          exit 1
        }
        retry_with_backoff 4 publish_bootstrap_alias_if_newer gcs "${gcs_bootstrap_latest_path}" "${gcs_bootstrap_latest_uri}" "${gcs_bootstrap_existing_path}" || {
          echo "failed uploading stable bootstrap alias to gcs after retries"
          exit 1
        }
      fi
      ;;
    dbfs)
      dbfs_storage_uri="${DBT_NOVA_PUBLISH_DBFS_PREFIX}/${storage_archive}"
      dbfs_manifest_uri="${DBT_NOVA_PUBLISH_DBFS_PREFIX}/${manifest_filename}"
      dbfs_metadata_uri="${DBT_NOVA_PUBLISH_DBFS_PREFIX}/${metadata_filename}"
      dbfs_bootstrap_uri="${DBT_NOVA_PUBLISH_DBFS_PREFIX}/${bootstrap_filename}"
      dbfs_bootstrap_latest_uri="${DBT_NOVA_PUBLISH_DBFS_PREFIX}/${bootstrap_latest_filename}"
      if [[ -n "${models_archive}" ]]; then
        dbfs_models_uri="${DBT_NOVA_PUBLISH_DBFS_PREFIX}/${models_archive}"
      fi
      dbfs_bootstrap_models_uri=""
      if [[ "${include_models_in_bootstrap}" == "true" ]]; then
        dbfs_bootstrap_models_uri="${dbfs_models_uri}"
      fi
      dbfs_bootstrap_path="${bootstrap_dir}/dbfs-${bootstrap_filename}"
      dbfs_bootstrap_latest_path="${bootstrap_dir}/dbfs-${bootstrap_latest_filename}"
      dbfs_bootstrap_existing_path="${bootstrap_dir}/dbfs-existing-${bootstrap_latest_filename}"
      write_bootstrap_contract \
        "${dbfs_bootstrap_path}" \
        "dbfs" \
        "${dbfs_manifest_uri}" \
        "${dbfs_storage_uri}" \
        "${dbfs_metadata_uri}" \
        "${dbfs_bootstrap_models_uri}"
      cp "${dbfs_bootstrap_path}" "${dbfs_bootstrap_latest_path}"
      if [[ "${publish_dry_run}" != "true" ]]; then
        if [[ -z "${DATABRICKS_HOST:-}" || -z "${DATABRICKS_ACCESS_TOKEN:-}" ]]; then
          echo "publish target dbfs requires DATABRICKS_HOST and DATABRICKS_ACCESS_TOKEN."
          exit 1
        fi
        retry_with_backoff 4 publish_to_dbfs "${storage_archive}" "${dbfs_storage_uri}" || {
          echo "failed uploading storage archive to dbfs after retries"
          exit 1
        }
        retry_with_backoff 4 publish_to_dbfs "${manifest_publish_path}" "${dbfs_manifest_uri}" || {
          echo "failed uploading manifest to dbfs after retries"
          exit 1
        }
        retry_with_backoff 4 publish_to_dbfs "${metadata_publish_path}" "${dbfs_metadata_uri}" || {
          echo "failed uploading metadata contract to dbfs after retries"
          exit 1
        }
        if [[ -n "${models_archive}" ]]; then
          retry_with_backoff 4 publish_to_dbfs "${models_archive}" "${dbfs_models_uri}" || {
            echo "failed uploading models archive to dbfs after retries"
            exit 1
          }
        fi
        retry_with_backoff 4 publish_to_dbfs "${dbfs_bootstrap_path}" "${dbfs_bootstrap_uri}" || {
          echo "failed uploading bootstrap contract to dbfs after retries"
          exit 1
        }
        retry_with_backoff 4 publish_bootstrap_alias_if_newer dbfs "${dbfs_bootstrap_latest_path}" "${dbfs_bootstrap_latest_uri}" "${dbfs_bootstrap_existing_path}" || {
          echo "failed uploading stable bootstrap alias to dbfs after retries"
          exit 1
        }
      fi
      ;;
    *)
      echo "unsupported publish target: ${target}"
      exit 1
      ;;
  esac

  if [[ -z "${published_targets_success}" ]]; then
    published_targets_success="${target}"
  else
    published_targets_success="${published_targets_success},${target}"
  fi
done

published_storage_uris="$(jq -cn \
  --arg s3 "${s3_storage_uri}" \
  --arg gcs "${gcs_storage_uri}" \
  --arg dbfs "${dbfs_storage_uri}" \
  '{} | if $s3 != "" then . + {s3: $s3} else . end
  | if $gcs != "" then . + {gcs: $gcs} else . end
  | if $dbfs != "" then . + {dbfs: $dbfs} else . end')"
published_manifest_uris="$(jq -cn \
  --arg s3 "${s3_manifest_uri}" \
  --arg gcs "${gcs_manifest_uri}" \
  --arg dbfs "${dbfs_manifest_uri}" \
  '{} | if $s3 != "" then . + {s3: $s3} else . end
  | if $gcs != "" then . + {gcs: $gcs} else . end
  | if $dbfs != "" then . + {dbfs: $dbfs} else . end')"
published_metadata_uris="$(jq -cn \
  --arg s3 "${s3_metadata_uri}" \
  --arg gcs "${gcs_metadata_uri}" \
  --arg dbfs "${dbfs_metadata_uri}" \
  '{} | if $s3 != "" then . + {s3: $s3} else . end
  | if $gcs != "" then . + {gcs: $gcs} else . end
  | if $dbfs != "" then . + {dbfs: $dbfs} else . end')"
published_bootstrap_uris="$(jq -cn \
  --arg s3 "${s3_bootstrap_uri}" \
  --arg gcs "${gcs_bootstrap_uri}" \
  --arg dbfs "${dbfs_bootstrap_uri}" \
  '{} | if $s3 != "" then . + {s3: $s3} else . end
  | if $gcs != "" then . + {gcs: $gcs} else . end
  | if $dbfs != "" then . + {dbfs: $dbfs} else . end')"
published_bootstrap_latest_uris="$(jq -cn \
  --arg s3 "${s3_bootstrap_latest_uri}" \
  --arg gcs "${gcs_bootstrap_latest_uri}" \
  --arg dbfs "${dbfs_bootstrap_latest_uri}" \
  '{} | if $s3 != "" then . + {s3: $s3} else . end
  | if $gcs != "" then . + {gcs: $gcs} else . end
  | if $dbfs != "" then . + {dbfs: $dbfs} else . end')"
published_models_uris="$(jq -cn \
  --arg s3 "${s3_models_uri}" \
  --arg gcs "${gcs_models_uri}" \
  --arg dbfs "${dbfs_models_uri}" \
  '{} | if $s3 != "" then . + {s3: $s3} else . end
  | if $gcs != "" then . + {gcs: $gcs} else . end
  | if $dbfs != "" then . + {dbfs: $dbfs} else . end')"

jq -n \
  --arg published_targets "${published_targets_success}" \
  --argjson published_storage_uris "${published_storage_uris}" \
  --argjson published_manifest_uris "${published_manifest_uris}" \
  --argjson published_metadata_uris "${published_metadata_uris}" \
  --argjson published_bootstrap_uris "${published_bootstrap_uris}" \
  --argjson published_bootstrap_latest_uris "${published_bootstrap_latest_uris}" \
  --argjson published_models_uris "${published_models_uris}" \
  '{
    published_targets: $published_targets,
    published_storage_uris: $published_storage_uris,
    published_manifest_uris: $published_manifest_uris,
    published_metadata_uris: $published_metadata_uris,
    published_bootstrap_uris: $published_bootstrap_uris,
    published_bootstrap_latest_uris: $published_bootstrap_latest_uris,
    published_models_uris: $published_models_uris
  }' > "${publish_summary_path}"

{
  echo "published_targets=${published_targets_success}"
  echo "publish_summary_path=${publish_summary_path}"
  echo "published_bootstrap_latest_uris=${published_bootstrap_latest_uris}"
  echo 'published_storage_uris_legacy={}'
  echo 'published_manifest_uris_legacy={}'
  echo 'published_metadata_uris_legacy={}'
  echo 'published_bootstrap_uris_legacy={}'
  echo 'published_models_uris_legacy={}'
} >> "$GITHUB_OUTPUT"

{
  echo "### Remote Publish"
  echo ""
  echo "- dry_run: \`${publish_dry_run}\`"
  echo "- targets: \`${published_targets_success}\`"
  echo "- models_distribution_mode: \`${DBT_NOVA_MODELS_DISTRIBUTION_MODE}\`"
  echo "- include_models_in_bootstrap: \`${include_models_in_bootstrap}\`"
  echo "- publish_summary_path: \`${publish_summary_path}\`"
  echo ""
  echo '```json'
  jq . "${publish_summary_path}"
  echo '```'
} >> "$GITHUB_STEP_SUMMARY"
