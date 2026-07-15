# Challenge Patterns

## Semantic-Layer Bypass

Flag semantic-layer bypass when all of these are true:

- the user question is KPI, metric, measure, rate, funnel, or conversion shaped
- the evidence packet shows a governed Nova metric, measure, recipe, or semantic
  parent that covers the question
- the draft uses a raw/source table, staging model, SQL inspection, or broad
  model search as its answer basis
- the draft does not state an evidence-backed fallback reason

Common evidence:

- `search_indicator` returned a relevant metric or measure
- `provenance.tier` is `semantic_layer` on a candidate parent
- context includes `meta.nova.measures`, `meta.nova.metrics`, or a recipe with a
  matching business definition
- the draft cites a source/staging relation such as `source.*`, `raw_*`, or
  `stg_*` as the primary answer basis

Acceptable fallback reasons:

- semantic discovery returned no relevant indicator
- the relevant indicator had no credible execution parent
- the question was not KPI-shaped after decomposition
- a recipe fully covered the recurring deliverable and supplied the semantic
  contract

Suggested fix:

- switch the answer to the governed entity or recipe, or
- keep the fallback but state the failed semantic-discovery attempt, why the
  governed source was not usable, and the caveat this creates

## Cross-Grain Raw Join Without Deterministic Surface

Flag an unsafe raw join when all of these are true:

- the user question is ratio, rate, funnel, or cross-grain KPI shaped
- Nova evidence marks the accepted indicator as `execution_surface:
  "metadata_only"`, `queryable: false`, or `direct_sql_queryable: false`
- agent modelling findings include cross-grain or deterministic-surface risks
  such as `ratio_like_metric_without_deterministic_surface` or
  `cross_grain_kpi_without_semantic_artifact`
- the draft joins raw/source fact tables from metric names, guessed keys, or
  broad model search instead of using or requesting a deterministic surface

Common evidence:

- `queryable_via: "none"`
- `category: "cross_grain_risk"`
- `execution_surface: "metadata_only"`
- `direct_sql_queryable: false`
- raw/source relations such as `source.*`, `raw_*`, or separate fact tables
  joined on inferred user, date, session, or order fields

Suggested fix:

- mark the answer `fix_required`
- state that Nova evidence made the indicator non-queryable for direct SQL
- ask for or propose a deterministic dbt relation, configured semantic-engine
  artifact, saved query, or recipe
- avoid inventing the raw fact join from metadata names alone

## Stale Or Unknown Freshness Without Caveat

Flag missing freshness caveats when:

- `provenance.freshness.status` is `stale` or `unknown`
- the draft uses that evidence as the basis for a result, conclusion, or
  recommendation
- the draft does not warn that freshness is stale or unknown

Common evidence:

- `freshness.status: stale`
- `freshness.status: unknown`
- `freshness.reason: no_freshness_timestamp`
- stale source freshness or manifest-generated timestamp beyond
  `stale_after_days`

Suggested fix:

- add a caveat with the exact freshness status and source
- include timestamp, age, and stale threshold when available
- avoid leadership-ready or launch-ready wording until freshness is resolved

## Non-Finding Cases

Do not flag a semantic bypass when the packet shows semantic discovery was tried
and no relevant governed source existed.

Do not flag a freshness caveat when the evidence is fresh and the answer does
not otherwise overstate readiness.

Do not flag a raw join when Nova evidence identifies a deterministic
relation-backed, semantic-engine, saved-query, or recipe surface for the same
KPI and the draft uses that surface with the documented caveats.

Do not invent a better source from names alone. Require actual Nova evidence.
