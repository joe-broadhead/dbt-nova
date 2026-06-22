# Negative Results Log

Use this log for experiments that did not improve Nova accuracy, latency,
maintainability, or agent behavior. Keep entries short, evidence-first, and
easy to scan during future ranking, retrieval, skill, and eval work.

Negative results are not failures. They are product evidence: they preserve the
reason a path was rejected so the project does not spend future time rediscovering
the same boundary.

## Entry Template

```markdown
## YYYY-MM-DD - Short Experiment Name

- Hypothesis:
- Eval suite:
- Change tested:
- Result:
- Decision:
- Related PR/issue:
```

Use `dbt-nova eval compare --before <DIR> --after <DIR>` when a before/after
suite exists, and paste the comparison summary or artifact path into `Result`.
If no eval suite exists yet, state what evidence was available and whether a new
eval should be added before revisiting the idea.

## 2026-06-22 - Raw Query Corpus Retrieval Is Not A Planning Source Of Truth

- Hypothesis: A corpus of raw historical queries could be a primary source for
  Nova's near-term agent-planning and retrieval strategy.
- Eval suite: Not run; this is a planning guardrail seeded before adding more
  retrieval or skill complexity.
- Change tested: Product direction based on raw query-corpus retrieval as a
  first-class source of truth.
- Result: Rejected for the near-term plan. Raw queries can contain stale,
  duplicated, idiosyncratic, or bypassed logic, and they do not reliably encode
  governed semantic intent.
- Decision: Prefer curated semantic metadata, `meta.nova` contracts, domain
  references, explicit eval evidence, and reviewed skills. Treat raw query
  corpora as optional supporting evidence only after a suite proves value.
- Related PR/issue: JOE-26
