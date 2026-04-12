# Metadata Audit

Nova includes a CLI-first metadata audit flow for CI gates and recurring
quality reports.

Use:

- `dbt-nova audit metadata-score`

This command loads a dbt manifest, scores selected entities with the existing
metadata scoring rubric, and produces:

- JSON report
- Markdown report
- required/advisory pass-fail gate result

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

When using `dbt_generate_manifest: true`, prefer the same secret-bundle pattern
as the Nova assets workflow so callers can work consistently across providers
and across same-owner or cross-owner reusable workflow calls:

```yaml
jobs:
  nova_metadata_audit:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-metadata-audit.yml@c443c5c301db04189fea690ff1adc32823721d11
    with:
      dbt_generate_manifest: true
      dbt_command_args_json: >-
        ["parse","--target","prod"]
      dbt_env_json: >-
        {"DBT_TARGET":"prod","DBT_PROFILES_DIR":"./"}
      dbt_secret_env_map_json: >-
        {"DBT_ACCESS_TOKEN":"DBT_ACCESS_TOKEN","DBT_BIGQUERY_KEYFILE_JSON":"DBT_BIGQUERY_KEYFILE_JSON"}
      selection_mode: changed
      resource_types_json: '["model"]'
      storage_instance_id: analytics-metadata-audit
    secrets:
      DBT_NOVA_SECRET_BUNDLE_JSON: ${{ secrets.DBT_NOVA_SECRET_BUNDLE_JSON }}
```

Secret resolution order for `dbt_secret_env_map_json` values:

1. keys in `DBT_NOVA_SECRET_BUNDLE_JSON`
2. inherited workflow secrets for same-owner calls

Use `DBT_NOVA_SECRET_BUNDLE_JSON` as the default integration pattern for
cross-owner reusable workflow calls or when you want one portable secret schema
across Databricks, BigQuery, DuckDB, and mixed-profile repos.
