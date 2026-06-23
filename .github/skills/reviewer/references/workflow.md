# Reviewer Workflow

Use this workflow to review a high-stakes draft analytics answer with explicit
Nova evidence.

## Deterministic Sequence

1. Confirm the packet includes the user question, draft answer, selected entity
   or source, semantic discovery or fallback evidence, provenance blocks, and
   SQL or recipe summary when applicable.
2. If evidence is missing, stop with `needs_evidence`.
3. Identify the evidence route used by the draft:
   - governed metric, measure, or recipe
   - curated model without direct indicator coverage
   - raw/source table fallback
4. Check whether a governed Nova metric, measure, or semantic parent was
   available for the same business concept.
5. Check each cited entity/source provenance block for `tier`, `readiness`, and
   `freshness.status`.
6. Compare the answer draft against the evidence:
   - Does the draft name the selected semantic definition?
   - Does it disclose raw-source fallback when used?
   - Does it caveat stale or unknown freshness?
   - Does it avoid overstating causality, precision, or readiness?
7. Return a verdict with findings, severity, evidence, suggested fix, and
   residual caveats.

## Evidence Completeness Rule

The minimum evidence for a pass is:

- selected entity/source identity
- semantic discovery result or explicit fallback reason
- provenance tier for the selected entity/source
- freshness status for evidence used in the answer, or explicit evidence that
  freshness is unavailable
- answer draft text

If any of those are missing, return:

```text
verdict: needs_evidence
findings:
- severity: high
  evidence: <missing field>
  suggested_fix: Provide <specific evidence> before finalizing.
```

## Review Boundaries

Do not rewrite the whole analysis. Give the smallest fix that lets the original
agent repair the final answer.

Do not replace reviewer evidence with intuition. If the draft feels wrong but
the evidence packet cannot prove it, ask for the missing search, context,
lineage, metadata-score, or freshness evidence.

Do not run `execute_sql`. If the result looks suspicious, ask for the executed
SQL summary, result rows, and filter validation evidence.

## Verdict Rule

Use `fix_required` when:

- a governed semantic candidate exists and the draft used raw/source table
  evidence without a fallback reason
- evidence freshness is `stale` or `unknown` and the draft omits a caveat
- the draft's selected entity, grain, time field, filter field, or indicator
  differs from the cited evidence

Use `needs_evidence` when the packet cannot prove pass or fail.

Use `pass` only when the draft's route, caveats, and evidence are consistent.
