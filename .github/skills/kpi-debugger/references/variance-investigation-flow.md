# Variance Investigation Flow

## Step 1: Define the variance question

State:
- current period
- comparison period
- absolute variance
- relative variance
- filters and breakdown

## Step 2: Validate comparison basis

Before interpreting the variance, confirm:
- both periods use the same grain
- both periods use the same filter set
- both periods use the same canonical KPI definition
- weekly and multi-week comparisons are weekday-aligned unless specified otherwise
- month, quarter, and year comparisons are calendar/date-aligned unless specified otherwise
- constant-currency comparisons follow the metric definition

## Step 3: Localize the variance

Break the variance into:
- time effect
- filter effect
- breakdown effect
- numerator / denominator effect for rate KPIs
- currency or FX effect when euro, local-currency, or constant-rate fields differ
- grain effect when a pre-aggregated surface is compared to a detail surface

## Step 4: Escalate only after localization

Use lineage or upstream freshness checks only after the variance has been localized to a specific field, dimension, or transformation stage.

If bounded reproduction cannot run, stop at a blocker statement and list the exact query contract or permissions needed to continue.
