# Chart Selection Matrix

## Use the simplest chart that matches the analytical question

| Question shape | Recommended pattern | Avoid when |
|---|---|---|
| single KPI at one point in time | metric card | the KPI needs distribution context |
| KPI over time | line chart | the grain is not ordered in time |
| ranked comparison across categories | sorted bar chart | category count is very high |
| part-to-whole at one point in time | stacked bar or narrow table | too many segments dilute interpretation |
| composition over time | stacked area or stacked bar | totals and categories both need exact reading |
| target vs actual | bullet or variance card | no target exists |
| distribution | histogram or box plot | only aggregated rows are available |
| entity or drill detail | table | the audience only needs a headline |

## Selection rules

- Use line charts only when the time grain is explicit and stable.
- Use bars for category comparison when exact rank matters.
- Use tables when the audience needs precise values across many dimensions.
- Do not use pies unless the category count is very small and part-to-whole is the only question.
- Avoid dual axes unless both units, scales, and interpretation risks are explicit.
- Cap ranked charts with a documented top-N and an "other" policy when categories are high-cardinality.
