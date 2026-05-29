---
description: Run the gated perf-autotune loop (search kernels, verify, push report to main)
---

Run one gated perf-autotune cycle for GlassSheet:

1. Execute `bash scripts/perf_autotune.sh 10000`. This searches ~10,000
   candidate/scenario measurements with `perf-lab`, verifies every candidate
   against the scalar reference, runs `fmt --check`, `clippy -D warnings`, and
   `cargo test --all`, then commits the refreshed `PERF_REPORT.md` and pushes to
   `main` — but only if all gates pass and the report changed.
2. Read the resulting `PERF_REPORT.md` and summarize: the verified champion, its
   speedup vs `scalar_ref`, and any candidate that FAILED correctness.
3. If a non-asm candidate beats the shipped `chunked4` kernel by a clear margin
   and passes correctness, propose (do not auto-apply) promoting it into
   `crates/engine/src/kernels.rs`, since that change must go through the full
   test gate and review.
4. If any gate failed, do NOT push; report what failed and stop.

To run this continuously: `/loop 30m /perf-autotune`.
