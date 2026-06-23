# Reviewer Output Template

```text
verdict: fix_required | needs_evidence | pass

findings:
- severity: blocker | high | medium | low
  title:
  evidence:
  suggested_fix:

residual_caveats:
- <caveat or none>
```

For `pass`, keep findings empty or include only low-severity wording
improvements. For `needs_evidence`, each finding should name one missing
evidence field and the exact source that should supply it.
