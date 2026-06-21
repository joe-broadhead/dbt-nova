# Metadata Audit

Nova includes a metadata audit flow for CI gates, MCP-connected agents, and
recurring quality reports.

Use:

- `dbt-nova audit metadata-score`
- `get_metadata_audit` MCP tool

This command loads a dbt manifest, scores selected entities with the existing
metadata scoring rubric, and produces:

- JSON report
- Markdown report
- required/advisory pass-fail gate result

Use `get_metadata_score` when you need a raw entity, column, or project metadata
score. Use `get_metadata_audit` when you need the higher-level audit contract:
selection modes, thresholds, required/advisory gate status, project summary,
entity rows, and report-ready JSON.

## Selection Modes

- `project`: score all selected resource types
- `changed`: score entities whose `original_file_path` or `patch_path` matches
  changed files
- `entities`: score explicit entity ids or unique names

## Common PR Gate

```bash
dbt-nova audit metadata-score \
  --selection-mode changed \
  --changed-files-json '["models/marts/orders.sql","models/marts/orders.yml"]' \
  --resource-types-json '["model"]' \
  --personas-json '["engineer","analyst","governance"]' \
  --thresholds-json '{"entity":{"engineer":{"min_score":70,"severity":"required"},"analyst":{"min_score":65,"severity":"advisory"},"governance":{"min_score":65,"severity":"advisory"}}}' \
  --manifest-path target/manifest.json \
  --report-json-path out/metadata-audit.json \
  --report-md-path out/metadata-audit.md \
  --json
```

## MCP Tool

```json
{"name":"get_metadata_audit","arguments":{"selection_mode":"changed","changed_files_json":"[\"models/marts/orders.sql\"]","resource_types_json":"[\"model\"]"}}
```

The MCP tool returns the same JSON report contract as the CLI audit command,
but does not write JSON/Markdown files and does not convert required threshold
failures into transport errors. Check `data.gate_status` and `data.summary` for
gate results.

## Threshold Contract

Thresholds are supplied as JSON.

Example:

```json
{
  "entity": {
    "engineer": { "min_score": 70, "severity": "required" },
    "analyst": { "min_score": 65, "severity": "advisory" },
    "governance": { "min_score": 65, "severity": "advisory" }
  },
  "project": {
    "engineer": { "min_score": 80, "severity": "required" }
  }
}
```

Rules:

- `severity: required` fails the command
- `severity: advisory` reports the miss without failing
- if `min_score` and `min_grade` are both set, both must pass
- `entity` thresholds apply to `changed` and `entities` selection modes
- `project` thresholds apply to the aggregate project score when
  `selection_mode=project`; the per-entity table remains informational in that
  mode

## Reports

JSON reports include:

- manifest hash/version
- selected entities
- per-persona scores
- per-category breakdowns
- recommendations
- gate summary

Markdown reports include:

- overall gate status
- scored target count
- pass/fail counts
- compact per-entity table
- top findings for failing entities

## CI Workflow

The repository also provides a reusable workflow:

- `.github/workflows/nova-metadata-audit.yml`

It follows the same installer and dbt invocation standards as the reusable
asset workflow, but disables vector, sparse, and reranker search so CI does not
pay for full search/model startup during metadata-only audits.

When `selection_mode: changed` and `changed_files_json` is omitted on
`pull_request` events, the reusable workflow resolves changed files from the
immutable PR event SHAs (`pull_request.base.sha` and `pull_request.head.sha`)
instead of the moving base branch name. This keeps reruns stable even after the
base branch has advanced. If the workflow cannot fetch those immutable commits,
it fails before running the audit rather than falling back to a moving branch
tip.

Changed-mode audits match dbt manifest entities by both `original_file_path`
and `patch_path`, so changing either a model SQL file or its YAML properties
file selects the model when the manifest records that path. If changed files
under `models/` look like dbt SQL or YAML model/schema files but no manifest
entity matches, Nova records a no-target advisory by default. Set
`fail_on_no_targets: true` to make that condition a required failure while still
emitting the JSON and Markdown audit reports.

When using `dbt_generate_manifest: true`, prefer the same secret-bundle pattern
as the Nova assets workflow so callers can work consistently across providers
and across same-owner or cross-owner reusable workflow calls:

```yaml
jobs:
  nova_metadata_audit:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-metadata-audit.yml@v0.0.5
    with:
      dbt_generate_manifest: true
      dbt_command_args_json: >-
        ["parse","--target","prod"]
      dbt_env_json: >-
        {"DBT_TARGET":"prod","DBT_PROFILES_DIR":"./"}
      dbt_secret_env_map_json: >-
        {"DBT_ACCESS_TOKEN":"DBT_ACCESS_TOKEN","DBT_BIGQUERY_KEYFILE_JSON":"DBT_BIGQUERY_KEYFILE_JSON"}
      selection_mode: changed
      # Optional. Omit on pull_request events to let the workflow resolve
      # immutable pull_request.base.sha...pull_request.head.sha changed files.
      # changed_files_json: '["models/marts/orders.sql","models/marts/orders.yml"]'
      resource_types_json: '["model"]'
      fail_on_no_targets: true
      storage_instance_id: analytics-metadata-audit
    secrets:
      DBT_NOVA_SECRET_BUNDLE_JSON: ${{ secrets.DBT_NOVA_SECRET_BUNDLE_JSON }}
```

Secret resolution order for `dbt_secret_env_map_json` values:

1. keys in `DBT_NOVA_SECRET_BUNDLE_JSON`
2. inherited workflow secrets for same-owner calls

Use `DBT_NOVA_SECRET_BUNDLE_JSON` as the default integration pattern for
cross-owner reusable workflow calls or when you want one portable secret schema
across Databricks, BigQuery, Snowflake, DuckDB, and mixed-profile repos.
