# Eval CI Templates

Nova evals fit best as two separate CI lanes:

- **Bridge evals** are deterministic Nova tool checks. Run them after dbt
  produces a manifest, and let mature suites block pull requests.
- **Provider-backed agent evals** execute an agent CLI such as OpenCode. Run
  them in private, scheduled, or manually dispatched workflows until the suite,
  provider, and trace capture are stable.

Use synthetic suite names and paths in shared examples. Keep project-specific
manifest URIs, provider output, raw traces, warehouse logs, and credentials out
of public artifacts.

For suite authoring and assertion details, see [Nova Evals](../features/evals.md).
For trace inspection and redaction behavior, see
[Trace Inspection And Redaction](../features/traces.md).

## Bridge Eval PR Gate

This workflow validates a suite, generates `target/manifest.json`, runs the
full bridge suite with telemetry, then checks the latest suite gate. The gate
step uses `jq` because `dbt-nova eval gate --json` reports `allowed` and
`blocked` but does not change the shell exit code by itself.

```yaml title=".github/workflows/nova-bridge-evals.yml"
name: Nova bridge evals

on:
  pull_request:
  workflow_dispatch:

permissions:
  contents: read

jobs:
  bridge_evals:
    runs-on: ubuntu-22.04
    timeout-minutes: 20
    env:
      DBT_NOVA_INSTALL_REF: master
      NOVA_EVAL_SUITE: evals/analyst-smoke.yml
      NOVA_EVAL_NAME: analyst-smoke
      NOVA_EVAL_OUTPUT: out/nova-bridge-evals
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-python@v5
        with:
          python-version: "3.11"

      - name: Install dbt project dependencies
        run: |
          python -m pip install --upgrade pip
          python -m pip install -r requirements.txt

      - name: Generate dbt manifest
        run: |
          dbt deps
          dbt parse --target ci

      - name: Install dbt-nova
        run: |
          curl -fsSL "https://raw.githubusercontent.com/joe-broadhead/dbt-nova/${DBT_NOVA_INSTALL_REF}/scripts/install.sh" | \
            bash -s -- --slim --non-interactive
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"

      - name: Validate eval suite
        run: |
          dbt-nova eval validate --suite "$NOVA_EVAL_SUITE"

      - name: Run bridge eval suite
        run: |
          mkdir -p "$NOVA_EVAL_OUTPUT"
          dbt-nova eval run \
            --suite "$NOVA_EVAL_SUITE" \
            --manifest-path target/manifest.json \
            --output-dir "$NOVA_EVAL_OUTPUT" \
            --telemetry \
            --telemetry-retention 1000 \
            --fail-under 1.0 \
            --json | tee "$NOVA_EVAL_OUTPUT/run.envelope.json"

      - name: Check latest eval gate
        run: |
          dbt-nova eval gate "$NOVA_EVAL_NAME" --json | tee "$NOVA_EVAL_OUTPUT/gate.envelope.json"
          jq -e '.data.allowed == true' "$NOVA_EVAL_OUTPUT/gate.envelope.json"

      - name: Publish bridge eval artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: nova-bridge-evals
          path: |
            out/nova-bridge-evals/**
            .nova/eval-runs/telemetry/*.jsonl
          if-no-files-found: warn
          retention-days: 14

      - name: Add eval card to summary
        if: always()
        run: |
          if [ -f "$NOVA_EVAL_OUTPUT/card.md" ]; then
            cat "$NOVA_EVAL_OUTPUT/card.md" >> "$GITHUB_STEP_SUMMARY"
          fi
```

Replace dependency installation and manifest generation with your normal dbt
CI setup. `master` tracks the current docs. For production, pin
`DBT_NOVA_INSTALL_REF` to a release tag or immutable commit that includes the
eval commands you use, and keep dbt warehouse credentials in GitHub secrets or
your usual dbt profile mechanism.

Run the full suite before `eval gate`. A filtered `--case-id` run, stale suite
file, missing suite hash, or telemetry retention value that trims the latest
run below its assertion count will block configured gates. Set
`--telemetry-retention` high enough for the largest full-suite run you want to
gate on. `NOVA_EVAL_NAME` must match the suite `name:` field, not the file path.

## Provider-Backed OpenCode Eval

Provider-backed evals are useful for scheduled regression evidence and release
reviews, but they depend on provider configuration, model availability, network
behavior, and trace capture. Keep them out of public default CI until they are
stable and the artifact policy has been reviewed.

In public repositories, keep provider-backed workflows disabled or upload only
reviewed summaries. Scheduled workflows still produce logs and artifacts that
may be visible to people with repository access.

This template is intentionally scheduled/manual and advisory. It records the
agent eval result, redacts any local Nova tool traces, uploads reports and
redacted traces, and writes the eval card to the job summary. Replace
`AGENT_PROVIDER_API_KEY` with the environment variable your OpenCode provider
actually reads.

```yaml title=".github/workflows/nova-agent-evals.yml"
name: Nova provider-backed evals

on:
  workflow_dispatch:
  schedule:
    - cron: "17 3 * * 1"

permissions:
  contents: read

jobs:
  opencode_agent_evals:
    runs-on: ubuntu-22.04
    timeout-minutes: 45
    env:
      DBT_NOVA_INSTALL_REF: master
      NOVA_AGENT_SUITE: evals/analyst-agent.yml
      NOVA_AGENT_NAME: analyst-agent-smoke
      NOVA_AGENT_OUTPUT: out/nova-agent-evals
      NOVA_AGENT_MODEL: opencode/deepseek-v4-flash-free
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-python@v5
        with:
          python-version: "3.11"

      - name: Install dbt project dependencies
        run: |
          python -m pip install --upgrade pip
          python -m pip install -r requirements.txt

      - name: Generate dbt manifest
        run: |
          dbt deps
          dbt parse --target ci

      - name: Install dbt-nova
        run: |
          curl -fsSL "https://raw.githubusercontent.com/joe-broadhead/dbt-nova/${DBT_NOVA_INSTALL_REF}/scripts/install.sh" | \
            bash -s -- --slim --non-interactive
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"

      - name: Validate agent eval suite
        run: |
          dbt-nova eval validate --suite "$NOVA_AGENT_SUITE"

      - name: Verify provider secret is available
        env:
          AGENT_PROVIDER_API_KEY: ${{ secrets.NOVA_AGENT_PROVIDER_API_KEY }}
        run: |
          if [ -z "${AGENT_PROVIDER_API_KEY:-}" ]; then
            echo "Missing NOVA_AGENT_PROVIDER_API_KEY secret for the selected provider."
            exit 1
          fi

      - name: Run OpenCode agent eval suite
        id: agent_eval
        continue-on-error: true
        env:
          AGENT_PROVIDER_API_KEY: ${{ secrets.NOVA_AGENT_PROVIDER_API_KEY }}
        run: |
          mkdir -p "$NOVA_AGENT_OUTPUT"
          dbt-nova eval agent run \
            --suite "$NOVA_AGENT_SUITE" \
            --provider opencode \
            --provider-model "$NOVA_AGENT_MODEL" \
            --manifest-path target/manifest.json \
            --output-dir "$NOVA_AGENT_OUTPUT" \
            --telemetry \
            --telemetry-retention 1000 \
            --timeout-secs 900 \
            --fail-under 0.9 \
            --json | tee "$NOVA_AGENT_OUTPUT/run.envelope.json"

      - name: Check advisory eval gate
        if: always()
        continue-on-error: true
        run: |
          if [ -d .nova/eval-runs/telemetry ]; then
            dbt-nova eval gate "$NOVA_AGENT_NAME" --json | tee "$NOVA_AGENT_OUTPUT/gate.envelope.json"
            jq -e '.data.allowed == true' "$NOVA_AGENT_OUTPUT/gate.envelope.json"
          else
            echo "No eval telemetry was written; inspect the agent eval output."
          fi

      - name: Redact local tool traces
        if: always()
        run: |
          mkdir -p "$NOVA_AGENT_OUTPUT/redacted-tool-calls"
          if compgen -G "$NOVA_AGENT_OUTPUT/tool-calls/*.jsonl" > /dev/null; then
            for trace in "$NOVA_AGENT_OUTPUT"/tool-calls/*.jsonl; do
              name="$(basename "$trace" .jsonl)"
              redacted="$NOVA_AGENT_OUTPUT/redacted-tool-calls/${name}.redacted.jsonl"
              dbt-nova trace redact \
                --path "$trace" \
                --out "$redacted" \
                --json > "$NOVA_AGENT_OUTPUT/redacted-tool-calls/${name}.redaction.json"
              dbt-nova trace summarize \
                --path "$redacted" \
                --report-md-path "$NOVA_AGENT_OUTPUT/redacted-tool-calls/${name}.redacted.md" \
                --json > "$NOVA_AGENT_OUTPUT/redacted-tool-calls/${name}.summary.json"
            done
          fi

      - name: Publish advisory eval artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: nova-agent-eval-reports
          path: |
            out/nova-agent-evals/results.json
            out/nova-agent-evals/results.tsv
            out/nova-agent-evals/card.md
            out/nova-agent-evals/report.md
            out/nova-agent-evals/suite.yml
            out/nova-agent-evals/run.envelope.json
            out/nova-agent-evals/gate.envelope.json
            out/nova-agent-evals/redacted-tool-calls/**
            .nova/eval-runs/telemetry/*.jsonl
          if-no-files-found: warn
          retention-days: 7

      - name: Add advisory summary
        if: always()
        run: |
          echo "OpenCode eval outcome: ${{ steps.agent_eval.outcome }}" >> "$GITHUB_STEP_SUMMARY"
          if [ -f "$NOVA_AGENT_OUTPUT/card.md" ]; then
            cat "$NOVA_AGENT_OUTPUT/card.md" >> "$GITHUB_STEP_SUMMARY"
          fi
          if [ "${{ steps.agent_eval.outcome }}" != "success" ]; then
            echo "" >> "$GITHUB_STEP_SUMMARY"
            echo "Provider-backed evals are advisory in this workflow. Inspect artifacts before treating the result as release-blocking evidence." >> "$GITHUB_STEP_SUMMARY"
          fi
```

If you promote a provider-backed eval to a blocking gate, remove
`continue-on-error`, keep the trace redaction and artifact steps under
`if: always()`, and confirm that failures do not print secrets, raw prompts,
private manifest URIs, or provider stdout/stderr into uploaded reports.

## Artifact Policy

Recommended bridge eval artifacts:

- `results.json`, `results.tsv`, `card.md`, `report.md`, and copied `suite.yml`
- `run.envelope.json` and `gate.envelope.json`
- telemetry JSONL under `.nova/eval-runs/telemetry/*.jsonl`

Recommended provider-backed artifacts:

- eval reports and telemetry as above
- redacted trace JSONL files and trace summaries
- no raw `tool-calls/*.jsonl`
- no provider stdout/stderr logs unless a reviewer has checked them locally

`results.json` can include assertion evidence, failure messages, selected
entity IDs, final-answer excerpts, and provider metadata. Review it before
publishing artifacts outside a private repository. Keep artifact retention short
for provider-backed jobs.

## Failure Policy

Use this rollout order:

1. Run bridge suites in advisory mode while authoring cases.
2. Add `--telemetry` and confirm `eval gate` sees full-suite telemetry.
3. Promote bridge suites to PR blockers with `jq -e '.data.allowed == true'`.
4. Run provider-backed evals privately on a schedule or by manual dispatch.
5. Promote provider-backed evals only after their provider configuration,
   trace capture, and artifact redaction behavior are stable.

Bridge evals are the normal merge gate. Provider-backed evals are slower
behavioral evidence and should usually remain advisory until the team has a
baseline for cost, latency, provider variance, and false failures.
