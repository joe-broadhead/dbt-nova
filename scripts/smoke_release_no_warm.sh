#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/smoke_release_no_warm.sh --manifest-path PATH [options]

Build and smoke-test the release binary against a manifest without vector,
sparse, or reranker warmup. The script writes JSON artifacts and a compact
Markdown report under --output-dir.

Required:
  --manifest-path PATH          Local dbt manifest.json to load.

Options:
  --binary PATH                 dbt-nova binary to test. Defaults to target/release/dbt-nova.
  --skip-build                  Do not run cargo build --release --locked.
  --output-dir PATH             Artifact directory. Defaults to target/nova-release-smoke/no-warm.
  --storage-dir PATH            Nova storage root. Defaults to <output-dir>/storage.
  --storage-instance-id ID      Storage instance id. Defaults to release-no-warm-smoke.
  --bridge-suite PATH           Optional bridge eval suite to validate/run.
  --agent-suite PATH            Optional agent eval suite to validate/run.
  --agent-providers CSV         Optional provider presets to run, for example opencode,claude,codex.
  --agent-timeout SECONDS       Provider eval timeout. Defaults to 600.
  --fail-under RATE             Eval fail-under threshold. Defaults to 1.0.
  -h, --help                    Show this help.

The script intentionally does not run `dbt-nova manifest warm` or enable the
MCP `warm_manifest` safety gate. `warm_manifest` is checked only as an expected
disabled tool-call response.
EOF
}

manifest_path=""
binary="target/release/dbt-nova"
binary_explicit=0
skip_build=0
output_dir="target/nova-release-smoke/no-warm"
storage_dir=""
storage_instance_id="release-no-warm-smoke"
bridge_suite=""
agent_suite=""
agent_providers=""
agent_timeout=600
fail_under="1.0"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest-path)
      manifest_path="${2:-}"
      [[ -n "$manifest_path" ]] || { echo "--manifest-path requires a path" >&2; exit 2; }
      shift 2
      ;;
    --binary)
      binary="${2:-}"
      [[ -n "$binary" ]] || { echo "--binary requires a path" >&2; exit 2; }
      binary_explicit=1
      shift 2
      ;;
    --skip-build)
      skip_build=1
      shift
      ;;
    --output-dir)
      output_dir="${2:-}"
      [[ -n "$output_dir" ]] || { echo "--output-dir requires a path" >&2; exit 2; }
      shift 2
      ;;
    --storage-dir)
      storage_dir="${2:-}"
      [[ -n "$storage_dir" ]] || { echo "--storage-dir requires a path" >&2; exit 2; }
      shift 2
      ;;
    --storage-instance-id)
      storage_instance_id="${2:-}"
      [[ -n "$storage_instance_id" ]] || { echo "--storage-instance-id requires a value" >&2; exit 2; }
      shift 2
      ;;
    --bridge-suite)
      bridge_suite="${2:-}"
      [[ -n "$bridge_suite" ]] || { echo "--bridge-suite requires a path" >&2; exit 2; }
      shift 2
      ;;
    --agent-suite)
      agent_suite="${2:-}"
      [[ -n "$agent_suite" ]] || { echo "--agent-suite requires a path" >&2; exit 2; }
      shift 2
      ;;
    --agent-providers)
      agent_providers="${2:-}"
      [[ -n "$agent_providers" ]] || { echo "--agent-providers requires a CSV list" >&2; exit 2; }
      shift 2
      ;;
    --agent-timeout)
      agent_timeout="${2:-}"
      [[ "$agent_timeout" =~ ^[0-9]+$ ]] || { echo "--agent-timeout requires an integer" >&2; exit 2; }
      shift 2
      ;;
    --fail-under)
      fail_under="${2:-}"
      [[ -n "$fail_under" ]] || { echo "--fail-under requires a rate" >&2; exit 2; }
      shift 2
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

[[ -n "$manifest_path" ]] || { echo "--manifest-path is required" >&2; usage >&2; exit 2; }
[[ -f "$manifest_path" ]] || { echo "manifest not found: $manifest_path" >&2; exit 2; }

mkdir -p "$output_dir"
manifest_path="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$manifest_path")"
output_dir="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$output_dir")"
storage_dir="${storage_dir:-${output_dir}/storage}"
mkdir -p "$storage_dir"
storage_dir="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$storage_dir")"

if [[ "$binary_explicit" -eq 0 ]]; then
  binary="$(
    cargo metadata --locked --format-version=1 --no-deps \
      | python3 -c 'import json,pathlib,sys; print(pathlib.Path(json.load(sys.stdin)["target_directory"]) / "release" / "dbt-nova")'
  )"
fi

smoke_env_remove=(
  DBT_NOVA_MANIFEST_URI
  DBT_NOVA_BOOTSTRAP_URI
  DBT_NOVA_STORAGE_ARTIFACT_URI
  DBT_NOVA_METADATA_ARTIFACT_URI
  DBT_NOVA_MODELS_ARTIFACT_URI
  DBT_NOVA_PRUNE_ALLOW_IDS
  DBT_NOVA_PRUNE_DENY_IDS
  DBT_NOVA_SERVER_TRANSPORT
  DBT_NOVA_TOOL_ALLOWLIST
  DBT_NOVA_TOOL_DENYLIST
  DBT_NOVA_TRACE_TOOL_CALLS_PATH
  DBT_NOVA_STORAGE_READ_ONLY
  DBT_NOVA_TOOL_RATE_LIMITS
  DBT_NOVA_TOOL_RATE_LIMIT_WINDOW_SECS
  DBT_NOVA_RESULT_PROFILE
  DBT_NOVA_MCP_RESULT_PROFILE
  DBT_NOVA_MCP_MAX_RESPONSE_BYTES
  DBT_NOVA_MCP_ENABLE_EVAL_RUN
  DBT_NOVA_MCP_ENABLE_EVAL_WRITES
  DBT_NOVA_MCP_ENABLE_AGENT_EVAL
  DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER
  DBT_NOVA_MCP_ENABLE_TRACE_WRITES
  DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD
  DBT_NOVA_MCP_ENABLE_MANIFEST_WARM
  DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN
)
unset "${smoke_env_remove[@]}"

export DBT_NOVA_SEARCH_ENABLE_VECTOR=false
export DBT_NOVA_SEARCH_ENABLE_SPARSE=false
export DBT_NOVA_SEARCH_ENABLE_RERANKER=false
export DBT_NOVA_SERVER_TRANSPORT=stdio
export DBT_NOVA_STORAGE_READ_ONLY=false
export DBT_NOVA_STORAGE_DIR="$storage_dir"
export DBT_NOVA_STORAGE_INSTANCE_ID="$storage_instance_id"

if [[ "$skip_build" -eq 0 ]]; then
  cargo build --release --locked
fi

[[ -x "$binary" ]] || { echo "binary is not executable: $binary" >&2; exit 2; }
binary="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$binary")"

"$binary" --version >"${output_dir}/version.txt"

"$binary" health check \
  --manifest-path "$manifest_path" \
  --json >"${output_dir}/health.json"

python3 - "${output_dir}/health.json" <<'PY'
import json, sys
payload = json.load(open(sys.argv[1], encoding="utf-8"))
data = payload.get("data", {})
if payload.get("status") != "success" or data.get("ready_for_traffic") is not True:
    raise SystemExit(f"health check did not reach ready_for_traffic=true: {payload}")
PY

"$binary" tool call show_metadata \
  --manifest-path "$manifest_path" \
  --storage-instance-id "$storage_instance_id" \
  --params-json '{}' \
  --json >"${output_dir}/tool-show-metadata.json"

"$binary" tool call search \
  --manifest-path "$manifest_path" \
  --storage-instance-id "$storage_instance_id" \
  --params-json '{"query":"orders","resource_types":["model"],"limit":3}' \
  --json >"${output_dir}/tool-search.json"

"$binary" tool call get_agent_readiness \
  --manifest-path "$manifest_path" \
  --storage-instance-id "$storage_instance_id" \
  --params-json '{"personas_json":"[\"analyst\"]"}' \
  --json >"${output_dir}/tool-agent-readiness.json"

warm_manifest_status=0
if env -u DBT_NOVA_MCP_ENABLE_MANIFEST_WARM "$binary" tool call warm_manifest \
  --manifest-path "$manifest_path" \
  --storage-instance-id "$storage_instance_id" \
  --params-json '{}' \
  --json >"${output_dir}/tool-warm-manifest-disabled.json" \
  2>"${output_dir}/tool-warm-manifest-disabled.stderr.log"; then
  echo "warm_manifest unexpectedly succeeded without DBT_NOVA_MCP_ENABLE_MANIFEST_WARM" >&2
  exit 1
else
  warm_manifest_status=$?
fi

python3 - "${output_dir}/tool-warm-manifest-disabled.json" "$warm_manifest_status" <<'PY'
import json, sys
path, status = sys.argv[1], int(sys.argv[2])
if status == 0:
    raise SystemExit("warm_manifest unexpectedly exited successfully")
payload = json.load(open(path, encoding="utf-8"))
error = payload.get("error") or {}
if payload.get("status") != "error" or error.get("error_code") != "INVALID_PARAMS":
    raise SystemExit(f"warm_manifest did not return the expected disabled-tool error: {payload}")
if "warm_manifest is disabled" not in str(error.get("error", "")):
    raise SystemExit(f"warm_manifest error did not mention the safety gate: {payload}")
PY

python3 - "$binary" "$manifest_path" "$storage_dir" "$storage_instance_id" "$output_dir/mcp-stdio.json" <<'PY'
import json
import os
import select
import subprocess
import sys
import time

binary, manifest_path, storage_dir, storage_instance_id, out_path = sys.argv[1:]
env = os.environ.copy()
for key in [
    "DBT_NOVA_MANIFEST_URI",
    "DBT_NOVA_BOOTSTRAP_URI",
    "DBT_NOVA_STORAGE_ARTIFACT_URI",
    "DBT_NOVA_METADATA_ARTIFACT_URI",
    "DBT_NOVA_MODELS_ARTIFACT_URI",
    "DBT_NOVA_PRUNE_ALLOW_IDS",
    "DBT_NOVA_PRUNE_DENY_IDS",
    "DBT_NOVA_SERVER_TRANSPORT",
    "DBT_NOVA_TOOL_ALLOWLIST",
    "DBT_NOVA_TOOL_DENYLIST",
    "DBT_NOVA_TRACE_TOOL_CALLS_PATH",
    "DBT_NOVA_STORAGE_READ_ONLY",
    "DBT_NOVA_TOOL_RATE_LIMITS",
    "DBT_NOVA_TOOL_RATE_LIMIT_WINDOW_SECS",
    "DBT_NOVA_RESULT_PROFILE",
    "DBT_NOVA_MCP_RESULT_PROFILE",
    "DBT_NOVA_MCP_MAX_RESPONSE_BYTES",
    "DBT_NOVA_MCP_ENABLE_EVAL_RUN",
    "DBT_NOVA_MCP_ENABLE_EVAL_WRITES",
    "DBT_NOVA_MCP_ENABLE_AGENT_EVAL",
    "DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER",
    "DBT_NOVA_MCP_ENABLE_TRACE_WRITES",
    "DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD",
    "DBT_NOVA_MCP_ENABLE_MANIFEST_WARM",
    "DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN",
]:
    env.pop(key, None)
env.update({
    "DBT_MANIFEST_PATH": manifest_path,
    "DBT_NOVA_STORAGE_DIR": storage_dir,
    "DBT_NOVA_STORAGE_INSTANCE_ID": storage_instance_id,
    "DBT_NOVA_SEARCH_ENABLE_VECTOR": "false",
    "DBT_NOVA_SEARCH_ENABLE_SPARSE": "false",
    "DBT_NOVA_SEARCH_ENABLE_RERANKER": "false",
    "DBT_NOVA_SERVER_TRANSPORT": "stdio",
    "DBT_NOVA_STORAGE_READ_ONLY": "false",
})
stderr_path = out_path.rsplit(".", 1)[0] + ".stderr.log"
stderr_handle = open(stderr_path, "w", encoding="utf-8")
try:
    proc = subprocess.Popen(
        [binary, "server", "start"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=stderr_handle,
        text=True,
        env=env,
    )
except Exception:
    stderr_handle.close()
    raise

next_id = 1

def send(payload):
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()

def read_response(request_id, timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        ready, _, _ = select.select([proc.stdout], [], [], 0.5)
        if not ready:
            continue
        line = proc.stdout.readline()
        if not line:
            break
        message = json.loads(line)
        if str(message.get("id")) == str(request_id):
            return message
    raise TimeoutError(f"timed out waiting for MCP response id={request_id}")

def request(method, params=None):
    global next_id
    request_id = next_id
    next_id += 1
    payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        payload["params"] = params
    send(payload)
    return read_response(request_id)

def call_tool(name, arguments):
    response = request("tools/call", {"name": name, "arguments": arguments})
    if "error" in response:
        raise RuntimeError(f"{name} returned JSON-RPC error: {response}")
    text = response["result"]["content"][0]["text"]
    return json.loads(text)

summary = {"checks": [], "tool_count": None, "ready_for_traffic": False}
try:
    request("initialize", {
        "protocolVersion": "2025-11-25",
        "capabilities": {},
        "clientInfo": {"name": "dbt-nova-release-smoke", "version": "1.0.0"},
    })
    send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

    tools = request("tools/list", {})["result"]["tools"]
    summary["tool_count"] = len(tools)
    if len(tools) != 53:
        raise RuntimeError(f"expected 53 MCP tools, got {len(tools)}")
    summary["checks"].append({"name": "tools/list", "ok": True})

    for _ in range(120):
        health = call_tool("health", {})
        if health.get("data", {}).get("ready_for_traffic") is True:
            summary["ready_for_traffic"] = True
            break
        time.sleep(0.25)
    if not summary["ready_for_traffic"]:
        raise RuntimeError("MCP health never reached ready_for_traffic=true")
    summary["checks"].append({"name": "health-ready", "ok": True})

    for name, arguments in [
        ("show_metadata", {}),
        ("search", {"query": "orders", "resource_types": ["model"], "limit": 3}),
        ("get_agent_readiness", {"personas_json": "[\"analyst\"]"}),
    ]:
        payload = call_tool(name, arguments)
        if payload.get("success") is not True:
            raise RuntimeError(f"{name} failed: {payload}")
        summary["checks"].append({"name": name, "ok": True})

    warm = call_tool("warm_manifest", {})
    if warm.get("success") is not False or warm.get("error_code") != "INVALID_PARAMS":
        raise RuntimeError(f"warm_manifest safety gate did not reject as expected: {warm}")
    summary["checks"].append({"name": "warm_manifest-disabled", "ok": True})
finally:
    proc.kill()
    proc.wait(timeout=10)
    stderr_handle.close()

with open(out_path, "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2, sort_keys=True)
PY

if [[ -n "$bridge_suite" ]]; then
  "$binary" eval validate --suite "$bridge_suite" >"${output_dir}/bridge-validate.txt"
  "$binary" eval run \
    --suite "$bridge_suite" \
    --manifest-path "$manifest_path" \
    --storage-instance-id "$storage_instance_id" \
    --output-dir "${output_dir}/bridge-eval" \
    --telemetry \
    --telemetry-retention 1000 \
    --fail-under "$fail_under" \
    --json >"${output_dir}/bridge-eval.json"
fi

if [[ -n "$agent_providers" ]]; then
  [[ -n "$agent_suite" ]] || { echo "--agent-suite is required with --agent-providers" >&2; exit 2; }
  "$binary" eval validate --suite "$agent_suite" >"${output_dir}/agent-validate.txt"
  IFS=',' read -r -a providers <<<"$agent_providers"
  for provider in "${providers[@]}"; do
    provider="${provider//[[:space:]]/}"
    [[ -n "$provider" ]] || continue
    "$binary" eval agent run \
      --suite "$agent_suite" \
      --provider "$provider" \
      --manifest-path "$manifest_path" \
      --storage-instance-id "${storage_instance_id}-${provider}" \
      --output-dir "${output_dir}/agent-${provider}" \
      --telemetry \
      --telemetry-retention 1000 \
      --timeout-secs "$agent_timeout" \
      --fail-under "$fail_under" \
      --json >"${output_dir}/agent-${provider}.json"
  done
fi

python3 - "$output_dir" "$manifest_path" "$storage_instance_id" <<'PY'
import json
import pathlib
import sys

output_dir = pathlib.Path(sys.argv[1])
manifest_path = sys.argv[2]
storage_instance_id = sys.argv[3]
health = json.load(open(output_dir / "health.json", encoding="utf-8"))
mcp = json.load(open(output_dir / "mcp-stdio.json", encoding="utf-8"))
summary = {
    "manifest_path": manifest_path,
    "storage_instance_id": storage_instance_id,
    "semantic_warmup": "not_run",
    "ready_for_traffic": health.get("data", {}).get("ready_for_traffic"),
    "mcp_tool_count": mcp.get("tool_count"),
    "artifacts_dir": str(output_dir),
}
with open(output_dir / "summary.json", "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2, sort_keys=True)
with open(output_dir / "report.md", "w", encoding="utf-8") as handle:
    handle.write("# dbt-nova no-warm release smoke\n\n")
    for key, value in summary.items():
        handle.write(f"- `{key}`: `{value}`\n")
PY

echo "No-warm release smoke passed. Artifacts: ${output_dir}"
