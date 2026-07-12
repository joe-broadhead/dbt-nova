# Module Size Ratchet

Nova uses a ratchet model for maintainability. The goal is not to chase small
files for its own sake; the goal is to keep hot paths and safety-sensitive
surfaces reviewable as the metadata bridge grows.

## Policy

The file-size policy in `CONTRIBUTING.md` applies to maintained source,
scripts, docs, and workflow files:

| Threshold | Meaning |
| --- | --- |
| `<= 1200` LOC | Soft target. Prefer incremental extraction when touching files above this line. |
| `> 1800` LOC | Hard review threshold. The file must have explicit rationale and a review date. |

Generated files, vendored dependencies, snapshots, fixture manifests, schema
JSON, and lockfiles are intentionally excluded from the ratchet.

## Check

Run:

```bash
scripts/check_module_size.sh
```

The check reads `module-size-exceptions.tsv`, counts tracked files with
`git ls-files`, reports every file above the soft target, and fails when:

* a file exceeds the hard threshold without an exception;
* an exception has missing metadata;
* an exception review date expires;
* an exception points to a file that no longer exceeds the hard threshold.

## Exception Register

`module-size-exceptions.tsv` is the checked-in exception register. Each row must
include:

| Field | Requirement |
| --- | --- |
| `path` | Tracked file path from the repository root. |
| `owner` | Responsible owner or owner group. |
| `review_by` | ISO date for the next review. |
| `reason` | Short rationale and intended split posture. |

Exceptions should be time-bounded. When a file drops below the hard threshold,
remove its row. When a file remains above the hard threshold, refresh the row
only after confirming the file still earns its size.

## Current Split Posture

The current hard-threshold files should be split only through narrow, tested
PRs that preserve public CLI/MCP contracts. Prioritize extractions around:

* hot paths such as manifest loading and search;
* security-sensitive provider/auth paths;
* high-churn CLI orchestration where behavior is already covered by contract
  tests.

Avoid broad rewrites. A smaller file that weakens Nova's metadata-bridge
contracts is a regression, not progress.
