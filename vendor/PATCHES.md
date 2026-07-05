# Vendored Crate Patches

This repository patches crates through `[patch.crates-io]` so release builds are
reproducible while upstream fixes settle. When upgrading either crate, compare
the vendored directory with the target upstream release, confirm each entry
below is upstreamed or intentionally retained, and run the linked tests.

## `vendor/hf-hub`

- **Upstream crate:** `hf-hub` `0.3.2`
- **Upstream repository:** `https://github.com/huggingface/hf-hub`
- **Local patch:** `src/api/sync.rs` and `src/api/tokio.rs` resolve relative
  redirect `Location` headers against the original request URL before following
  the redirected model download.
- **Rationale:** model hosting/CDN paths can return relative redirects. The
  upstream sync path treated those as invalid absolute URLs, which broke online
  embedding model downloads.
- **Covering test:** `tests/hf_hub_relative_redirect.rs`
- **Upgrade rule:** remove the local patch only after the target upstream
  `hf-hub` release handles relative redirects in both sync and async clients.

## `vendor/fastembed`

- **Upstream crate:** `fastembed` `3.14.1`
- **Upstream repository:** `https://github.com/Anush008/fastembed-rs`
- **Local patch:** vendored to keep the `fastembed` dependency graph aligned
  with the pinned `ort-sys` release-candidate stack used by dbt-nova.
- **Rationale:** dbt-nova release binaries need a stable ONNX Runtime/ort
  combination across Linux and macOS targets.
- **Covering checks:** full feature CI, macOS smoke CI, release binary smoke,
  and `cargo deny check`.
- **Upgrade rule:** upgrade `fastembed`, `hf-hub`, `ort`, and `ort-sys`
  together; remove the vendor patch only after the stable upstream stack passes
  the release-grade checks.
