# Agent Readiness Audit

`dbt-nova audit agent-readiness` produces a manifest-level readiness report for
agent workflows. It combines project metadata scores, entity-level metadata
signals, Nova indicator metadata, and optional eval gate evidence into one JSON
and Markdown artifact.

Use it before enabling a manifest for production agent analysis, launch reviews,
or recurring CI evidence.

MCP clients can request the same JSON report with `get_agent_readiness`.
The MCP tool accepts inline `personas_json`, `thresholds_json`, and
`eval_gate_json`, and returns the report without writing files or applying CLI
exit semantics.

## Local Command

```bash
mkdir -p out

dbt-nova audit agent-readiness \
  --manifest-path target/manifest.json \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --json
```

The command is metadata-only by default. It loads the manifest with vector,
sparse, and reranker search disabled, even when those search features are
enabled in the surrounding environment.

Use a stable `--storage-instance-id` in repeatable automation so reruns share
the same manifest index cache. Add `--cleanup-storage-on-start` when you want a
fresh local cache for the manifest under test.

## Inputs

- `--manifest-path` or `--manifest-uri`: manifest or prebuilt artifact source
- `--storage-instance-id`: stable storage instance for cached manifest indexes
- `--cleanup-storage-on-start`: remove the selected storage instance before
  loading
- `--read-only`: reuse already materialized storage without writes
- `--personas-json`: personas to score, defaulting to
  `["engineer","analyst","governance"]`
- `--thresholds-json` or `--thresholds-file`: advisory or required readiness
  thresholds
- `--eval-gate-json` or `--eval-gate-file`: output from
  `dbt-nova eval gate <SUITE> --json`
- `--report-json-path` and `--report-md-path`: report artifact destinations
- `--fail-on-blockers`: exit non-zero when blocking findings are present
- `--json`: print a CLI envelope containing the report

## CI Workflow

Run agent readiness after dbt has produced `target/manifest.json`. This example
starts in advisory mode, writes machine and human reports, always uploads the
reports as workflow artifacts, and mirrors the Markdown report into the job
summary. Replace `<nova-release-tag>` with a release that includes
`audit agent-readiness`; until that release exists, install Nova from an
immutable source commit instead of using the release installer step.

```yaml
name: Agent readiness

on:
  pull_request:
  workflow_dispatch:

jobs:
  agent_readiness:
    runs-on: ubuntu-22.04
    env:
      DBT_NOVA_RELEASE: <nova-release-tag>
      READINESS_THRESHOLDS: >-
        {"overall":{"min_score":70,"severity":"advisory"},"persona":{"engineer":{"min_score":70,"severity":"advisory"},"analyst":{"min_score":65,"severity":"advisory"},"governance":{"min_score":65,"severity":"advisory"}}}
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-python@v5
        with:
          python-version: "3.11"

      - name: Install cosign
        uses: sigstore/cosign-installer@398d4b0eeef1380460a10c8013a76f728fb906ac
        with:
          cosign-release: "v2.4.1"

      - name: Install dbt project dependencies
        run: |
          python -m pip install --upgrade pip
          python -m pip install -r requirements.txt

      - name: Generate dbt manifest
        run: |
          dbt deps
          dbt parse --target prod

      - name: Install dbt-nova
        run: |
          curl -fsSL "https://raw.githubusercontent.com/joe-broadhead/dbt-nova/${DBT_NOVA_RELEASE}/scripts/install.sh" | \
            DBT_NOVA_VERSION="${DBT_NOVA_RELEASE}" DBT_NOVA_VERIFY_SIGNATURE=1 bash -s -- --slim --non-interactive
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"

      - name: Run agent-readiness report
        run: |
          mkdir -p out
          dbt-nova audit agent-readiness \
            --manifest-path target/manifest.json \
            --storage-instance-id "agent-readiness-${{ github.event.pull_request.number || github.run_id }}" \
            --thresholds-json "$READINESS_THRESHOLDS" \
            --report-json-path out/agent-readiness.json \
            --report-md-path out/agent-readiness.md \
            --json | tee out/agent-readiness.envelope.json

      - name: Publish readiness artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: agent-readiness
          path: |
            out/agent-readiness.json
            out/agent-readiness.md
            out/agent-readiness.envelope.json
          if-no-files-found: error

      - name: Add readiness summary
        if: always()
        run: |
          if [ -f out/agent-readiness.md ]; then
            cat out/agent-readiness.md >> "$GITHUB_STEP_SUMMARY"
          fi
```

Replace the dependency installation and dbt command with the same setup your
project uses in CI, such as `dbt compile`, `dbt parse`, or `dbt build`. Keep
database credentials in GitHub secrets or your normal dbt profile mechanism;
the readiness command only needs the generated manifest. For production
workflows, pin GitHub Action refs and the dbt-nova installer URL to a release
tag or immutable commit.

## Thresholds And Blocking

Thresholds are optional JSON. Default score thresholds are advisory; modelling
blockers are treated as true blockers, while high-severity modelling count
thresholds remain advisory so the command can produce evidence without making
all modelling findings CI-blocking.

```json
{
  "overall": { "min_score": 70, "severity": "advisory" },
  "persona": {
    "engineer": { "min_score": 70, "severity": "advisory" },
    "analyst": { "min_score": 65, "severity": "advisory" },
    "governance": { "min_score": 65, "severity": "advisory" }
  },
  "modelling": {
    "max_blockers": { "value": 0, "severity": "required" },
    "max_high": { "value": 10, "severity": "advisory" }
  }
}
```

Set `severity` to `required` for a threshold that should create a blocker.
`--fail-on-blockers` turns blockers into a failing CLI exit after reports have
been written.
Modelling thresholds can also be supplied through
`DBT_NOVA_AGENT_READINESS_MODELLING_MAX_BLOCKERS`,
`DBT_NOVA_AGENT_READINESS_MODELLING_MAX_HIGH`, and their `*_REQUIRED` flags.

The recommended rollout is:

- first run advisory thresholds and publish reports without
  `--fail-on-blockers`
- use several runs to establish the project baseline and fix noisy metadata
  gaps
- tighten selected thresholds to `required`
- add `--fail-on-blockers` once the team wants readiness regressions to block
  merges or releases

Example required gate after baseline:

```bash
dbt-nova audit agent-readiness \
  --manifest-path target/manifest.json \
  --thresholds-json '{"overall":{"min_score":80,"severity":"required"},"persona":{"default":{"min_grade":"B","severity":"required"}}}' \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --fail-on-blockers \
  --json
```

## Readiness Bands And Blockers

The report's `readiness_band` is deterministic:

- `blocked`: one or more blocking findings are present
- `high`: no blockers and `overall_score >= 85`
- `medium`: no blockers and `70 <= overall_score < 85`
- `low`: no blockers and `overall_score < 70`

The report's `gate_status` is `fail` when blockers exist, `advisory` when only
improvement findings exist, and `pass` when neither blockers nor material
improvements are detected.

Blocking findings come from:

- required `overall_score` threshold misses (`overall_threshold_missed`)
- required persona threshold misses (`persona_threshold_missed`)
- deterministic agent-modelling blocker findings (`agent_modelling_blocker`)
- required agent-modelling count threshold misses
- blocked eval gate evidence (`eval_gate_blocked`)

Advisory improvement findings can include missed advisory thresholds, missing
or unavailable eval gate evidence, low-scoring entity metadata, signal gaps such
as missing owners or primary-key evidence, ambiguous indicator metadata, and
high/medium agent-modelling findings.
Missing eval evidence is a next action by default; it is not a blocker unless a
provided eval gate report is blocked.

## Eval Gate Evidence

Agent readiness can include the current eval gate status:

```bash
dbt-nova eval gate analyst-smoke --json > out/eval-gate.json

dbt-nova audit agent-readiness \
  --manifest-path target/manifest.json \
  --eval-gate-file out/eval-gate.json \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --fail-on-blockers \
  --json
```

The readiness command accepts either the raw gate report or the full CLI
envelope emitted by `eval gate --json`. A blocked eval gate is recorded as a
readiness blocker. Missing eval gate evidence is recorded as a next action, not
as a blocker.

See [Evals](evals.md#readiness-gates) for gate telemetry rules and
[Metadata Audit](metadata-audit.md) for changed-entity metadata gates that fit
PR-only checks.

## Metadata-Only Scope And Limitations

Agent readiness is a manifest and metadata quality signal. It does not execute
warehouse SQL, call an LLM provider, or prove that an agent's final answer is
correct. Treat it as launch evidence for discoverability, metadata coverage,
indicator clarity, and optional eval gate freshness.

Use metadata-only readiness when you want a fast report from `manifest.json` in
PRs, release checks, or recurring metadata reviews. Use
`dbt-nova eval run` for bridge eval assertions against Nova tool results, and
use `dbt-nova eval agent run` only for slower provider-backed smoke tests where
you need to verify how a configured agent uses the tools. Use warehouse or
provider-specific checks separately when the question depends on live data,
credentials, SQL execution, or generated answer quality.

## Report Contract

JSON reports use `schema_version: "agent_readiness.v1"` and include:

- sanitized manifest source, hash, version, entity counts, and search readiness
- personas, selected resource types, thresholds, storage mode, and metadata-only
  mode
- `scoring_contract` with the metadata score grade bands, description tiers,
  array tiers, canonical grain shape, and primary-key integrity evidence rules
- overall readiness score, grade, readiness band, and gate status
- per-persona project metadata scores and threshold status
- compact triage fields in `summary`: score/grade buckets, worst entities by
  persona, category weak spots, repeated recommendation fields, estimated point
  impact, agent-modelling counts/top codes, and drill-down hints
- blocking findings and improvement findings
- lowest-scoring or signal-poor entity findings with top recommendations and
  metadata score diagnostics
- ambiguous Nova indicator definitions that need stronger execution metadata
- advisory `suggested_meta_patches` for missing or weak `meta`, `meta.nova`,
  and column metadata fields
- draft `golden_question_seeds` that can be reviewed before copying into eval
  suites
- eval gate status, if supplied
- ordered next actions

Markdown reports contain the same evidence in a compact review format for PR
comments, release notes, or CI job summaries.

## Report Artifacts

Write both report files in CI:

- `agent-readiness.json` is the machine contract for bots, dashboards, and
  follow-up automation
- `agent-readiness.md` is the review artifact for PR comments, release notes,
  and `GITHUB_STEP_SUMMARY`
- `agent-readiness.envelope.json`, when captured from `--json`, preserves the
  CLI success/error envelope used in logs

Upload reports with `if: always()` so reviewers can inspect readiness evidence
even when a later required gate fails. Do not commit generated reports. Treat
manifest sources, original file paths, and metadata values as project evidence:
avoid printing private manifest URIs or secrets in surrounding CI logs.

## Using Output To Create Work

Use `next_actions` as the ordered triage list. Promote items into remediation
issues or PR tasks based on their `category`, `priority`, and evidence.

Use `suggested_meta_patches` as reviewable dbt YAML work, not as automatic
edits. Assign owner, grain, primary-key, sensitivity, and indicator metadata
placeholders to the people who can supply real project truth.

Use `golden_question_seeds` as an eval backlog starter. Review each seed,
replace placeholders or date-sensitive wording, then copy approved cases into
an eval suite. Bridge seeds can become `eval run` checks; manual-review seeds
should become human-authored cases before they block CI.

Use `summary.drill_down_hints` to fetch detailed metadata-score rows only for
the weakest entities or personas. That keeps reports compact while preserving a
path to deeper analysis with `get_metadata_score`.

## Remediation Suggestions

The report includes `suggested_meta_patches` when readiness gaps point to
reviewable dbt metadata improvements. These are data suggestions, not automatic
file edits. Each suggestion includes:

- a stable `id`
- target entity, optional column, optional indicator name/type, and source path
- `field_path`, such as `meta.owner`, `meta.nova.grain.primary_key`, or
  `columns.order_id.meta.primary_key`
- `suggested_value`
- `placeholder`, `rationale`, `severity`, `confidence`, and evidence

When Nova lacks enough evidence, suggestions use explicit placeholders such as
`__OWNER_OR_TEAM__`, `__PRIMARY_KEY_COLUMN__`, `__SENSITIVITY__`, or
`__EXPRESSION_OR_FIELD__`. Reviewers should replace placeholders with real
project metadata before committing dbt YAML changes.

Suggestions stay conservative. For example, missing ownership produces a
placeholder owner suggestion rather than a fabricated person, and ambiguous
metrics ask for expression/canonical clarification rather than inventing ground
truth.

Suggestions consume metadata score diagnostics where available. For example, a
string-valued `meta.nova.grain` produces an `invalid_grain_shape` diagnostic and
a whole-object `meta.nova.grain` suggestion using the canonical
`primary_key`/`time_field`/`dimensions` shape, rather than suggesting unsafe
nested edits under a non-object value.

## Golden-Question Seeds

The report also includes `golden_question_seeds` to help turn readiness gaps
into an eval backlog. Seeds are draft cases: review and adapt them before using
them as CI gates.

Each seed includes:

- a stable `id`
- `seed_type`: `bridge`, `agent`, or `manual_review`
- target persona and question/task text
- expected entity and indicator IDs when Nova has enough evidence
- recommended assertion data, such as `search_indicator_rank` for canonical
  metric discovery or `metadata_score_min` for review gates
- rationale, `review_required`, and `date_policy`

Canonical metrics or measures with execution metadata can produce bridge seeds.
Ambiguous or missing metric metadata produces manual-review seeds instead of
false ground truth. Generated questions avoid relative dates; date-sensitive
cases should be anchored manually with explicit dates before becoming CI gates.

## MCP Tool

```json
{"name":"get_agent_readiness","arguments":{"personas_json":"[\"engineer\",\"analyst\"]"}}
```

To include eval gate evidence, pass either the raw gate report or the full
`eval gate --json` CLI envelope as `eval_gate_json`.

See also:

- [MCP and CLI parity](../api/mcp-cli-parity.md)
- [MCP tool reference](../api/tools.md#get_agent_readiness)
- [Metadata Audit](metadata-audit.md)
- [Evals](evals.md)
