# Eval Author Workflow

## Choose The Eval Layer

Use bridge evals when the question is:
- Does search return the canonical entity?
- Does indicator discovery find the right measure or metric?
- Does context expose the fields an agent needs?
- Does lineage contain the expected dependency or consumer?
- Do recipes exist and expose query metadata?
- Does metadata score meet the minimum quality bar?

Use agent evals when the question is:
- Did the agent call the right Nova tools?
- Did it avoid unsafe or premature tools such as `execute_sql`?
- Did it call discovery before context or execution?
- Did selected entity evidence appear in the tool trace?
- Did the final answer cite the intended concept without leaking implementation detail?

## Ground Truth Discovery

Establish expected values before writing YAML:

```bash
dbt-nova tool call search_indicator --params-json '{"query":"<business term>","limit":5}' --json
dbt-nova tool call get_context --params-json '{"id_or_name":"<unique_id>","include_sql":false}' --json
dbt-nova tool call search_columns --params-json '{"query":"<field term>","limit":5}' --json
dbt-nova tool call get_metadata_score --params-json '{"id_or_name":"<unique_id>","persona":"analyst","include_breakdown":false}' --json
```

Prefer MCP for the same discovery when the client has Nova MCP tools installed.
When using the CLI, pass the same `--manifest-path` or `--manifest-uri` that the
suite will use. Do not add `--read-only` during first-time discovery unless a
reusable index is already materialized; otherwise Nova cannot build the index
needed to answer search tools.

For large manifests or memory-constrained machines, explicitly disable vector,
sparse, and reranker search when the suite is not testing semantic model
behavior. Do not run `manifest warm` or `warm_manifest` just to validate bridge
or provider tool-use behavior.

When using MCP, wait for `health.data.ready_for_traffic=true` before recording
ground truth. `INDEX_BUILDING` is startup evidence, not a failed assertion.

Freeze relative dates before writing agent tasks. For example, replace "last
week" with explicit start/end dates and state the comparison basis. If the eval
targets a recipe, follow that recipe's calendar contract instead of assuming a
generic week boundary.

## Authoring Loop

1. Create or update the suite.
2. Run `dbt-nova eval validate --suite <suite>`.
3. Run one bridge case with `dbt-nova eval run --suite <suite> --case-id <id>`.
4. Fix expected ids, ranks, or metadata gaps.
5. Run the full bridge suite with `--telemetry` when the suite will be readiness-gated.
6. Add agent cases for workflows where tool-use behavior matters.
7. Run one agent case with the target provider for iteration; run the full agent suite with `--telemetry` before checking a readiness gate.
8. Inspect `tool-calls/<case>.jsonl`, `stdout.log`, and `report.md`.
9. Set the CI gate only after the suite has at least one clean local run.
10. For high-stakes, launch-readiness, or recurring production suites, check `dbt-nova eval gate <suite_name> --json` after full-suite telemetry-producing runs.

## Gate Selection

Use strict gates for smoke suites:
- `--fail-under 1.0`
- `gate.threshold: 1.0`
- small number of high-signal cases
- should run after every manifest build

Use realistic gates for broad regression suites:
- `--fail-under 0.90` to `0.98`
- `gate.threshold: 0.90` to `0.98`
- larger coverage across domains
- should run on a schedule or before metadata releases

Do not lower a gate to hide a deterministic failure. Lower only when the suite intentionally includes exploratory or provider-sensitive agent behavior.
