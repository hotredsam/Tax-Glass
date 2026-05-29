//! GlassSheet perf-lab — an autotuning harness for the engine's numeric
//! reduction kernels.
//!
//! It procedurally generates a corpus of inputs and a set of candidate `sum`
//! implementations (scalar, chunked/autovectorized, an AVX intrinsics path, and
//! a **literal inline-assembly** path on x86_64), then:
//!   1. **verifies** every candidate against the strict left-to-right scalar
//!      reference over an edge-case corpus (NaN/Inf/-0/denormals/empty/…); a
//!      candidate that disagrees is recorded as FAILED and excluded from
//!      selection — never silently shipped;
//!   2. **measures** the survivors across input-size scenarios (median of K
//!      trials), reaching ~`--iterations` total measurements;
//!   3. writes a Markdown report (default `PERF_REPORT.md`) with the machine,
//!      detected CPU features, a candidate × scenario leaderboard, and the
//!      verified champion.
//!
//! The harness only ever *reports*; promoting a champion into the engine is a
//! separate, gated step (see `scripts/perf_autotune.sh`).

use clap::Parser;
use std::time::Instant;

type SumFn = fn(&[f64]) -> f64;

struct Candidate {
    name: &'static str,
    kind: &'static str,
    f: SumFn,
    available: bool,
}

#[derive(Parser, Debug)]
#[command(
    name = "perf-lab",
    about = "Autotune + verify GlassSheet math kernels",
    version
)]
struct Cli {
    /// Approximate cap on total (candidate × scenario × trial) measurements.
    #[arg(long, default_value_t = 10_000)]
    iterations: usize,
    /// Deterministic seed for the input corpus.
    #[arg(long, default_value_t = 0x9E37_79B9)]
    seed: u64,
    /// Markdown report output path.
    #[arg(long, default_value = "PERF_REPORT.md")]
    out: String,
}

// ---------------------------------------------------------------------------
// Candidate kernels
// ---------------------------------------------------------------------------

/// Strict left-to-right reference — the correctness oracle.
fn scalar_ref(xs: &[f64]) -> f64 {
    let mut s = 0.0;
    for &x in xs {
        s += x;
    }
    s
}

/// The engine's shipped kernel (4 accumulators, autovectorizes).
fn chunked4(xs: &[f64]) -> f64 {
    glasssheet_engine::kernels::sum(xs)
}

/// Eight independent accumulators.
fn chunked8(xs: &[f64]) -> f64 {
    let mut acc = [0.0f64; 8];
    let mut chunks = xs.chunks_exact(8);
    for c in &mut chunks {
        for (a, &x) in acc.iter_mut().zip(c) {
            *a += x;
        }
    }
    let mut total =
        ((acc[0] + acc[1]) + (acc[2] + acc[3])) + ((acc[4] + acc[5]) + (acc[6] + acc[7]));
    for &x in chunks.remainder() {
        total += x;
    }
    total
}

#[cfg(target_arch = "x86_64")]
fn intrin_avx(xs: &[f64]) -> f64 {
    if !std::is_x86_feature_detected!("avx") {
        return chunked4(xs);
    }
    // SAFETY: guarded by the runtime AVX check above.
    unsafe { intrin_avx_impl(xs) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn intrin_avx_impl(xs: &[f64]) -> f64 {
    use core::arch::x86_64::*;
    let mut acc = _mm256_setzero_pd();
    let mut chunks = xs.chunks_exact(4);
    for c in &mut chunks {
        acc = _mm256_add_pd(acc, _mm256_loadu_pd(c.as_ptr()));
    }
    // Horizontal sum of the 4 lanes.
    let hi = _mm256_extractf128_pd(acc, 1);
    let lo = _mm256_castpd256_pd128(acc);
    let s = _mm_add_pd(lo, hi);
    let shuf = _mm_unpackhi_pd(s, s);
    let mut total = _mm_cvtsd_f64(_mm_add_sd(s, shuf));
    for &x in chunks.remainder() {
        total += x;
    }
    total
}

/// A **literal inline-assembly** scalar SSE2 accumulation (`addsd` loop). Same
/// addition order as the scalar reference, so it must match it exactly — a
/// correctness baseline for the asm path that more aggressive asm variants build
/// on.
#[cfg(target_arch = "x86_64")]
fn asm_scalar_sse(xs: &[f64]) -> f64 {
    let ptr = xs.as_ptr();
    let len = xs.len();
    let mut acc: f64;
    // SAFETY: reads `len` f64s from `ptr` (the slice), no writes, no stack use.
    unsafe {
        core::arch::asm!(
            "xorpd {acc}, {acc}",
            "xor {i}, {i}",
            "2:",
            "cmp {i}, {len}",
            "jae 3f",
            "addsd {acc}, qword ptr [{ptr} + {i}*8]",
            "inc {i}",
            "jmp 2b",
            "3:",
            ptr = in(reg) ptr,
            len = in(reg) len,
            i = out(reg) _,
            acc = out(xmm_reg) acc,
            options(readonly, nostack),
        );
    }
    acc
}

fn candidates() -> Vec<Candidate> {
    let mut v = vec![
        Candidate {
            name: "scalar_ref",
            kind: "scalar",
            f: scalar_ref,
            available: true,
        },
        Candidate {
            name: "chunked4 (shipped)",
            kind: "autovec",
            f: chunked4,
            available: true,
        },
        Candidate {
            name: "chunked8",
            kind: "autovec",
            f: chunked8,
            available: true,
        },
    ];
    #[cfg(target_arch = "x86_64")]
    {
        let has_avx = std::is_x86_feature_detected!("avx");
        v.push(Candidate {
            name: "intrin_avx",
            kind: "intrinsics",
            f: intrin_avx,
            available: has_avx,
        });
        v.push(Candidate {
            name: "asm_scalar_sse",
            kind: "inline-asm",
            f: asm_scalar_sse,
            available: true,
        });
    }
    v
}

// ---------------------------------------------------------------------------
// Corpus, verification, measurement
// ---------------------------------------------------------------------------

/// Tiny deterministic xorshift so the corpus is reproducible without a dep.
struct Rng(u64);
impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        // Map to roughly [-1, 1).
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

/// Numeric scenarios used for *speed* (no NaN/Inf so timing is clean).
fn speed_scenarios(seed: u64) -> Vec<(String, Vec<f64>)> {
    let mut rng = Rng(seed | 1);
    [8usize, 64, 1_000, 16_384, 100_000]
        .iter()
        .map(|&n| {
            let data: Vec<f64> = (0..n).map(|_| rng.next_f64()).collect();
            (format!("len {n}"), data)
        })
        .collect()
}

/// Inputs used for *correctness*, including nasty edge cases.
fn correctness_inputs(seed: u64) -> Vec<Vec<f64>> {
    let mut rng = Rng(seed | 1);
    let mut inputs: Vec<Vec<f64>> = vec![
        vec![],
        vec![0.0],
        vec![-0.0, 0.0],
        vec![1.0, 2.0, 3.0],
        vec![f64::NAN, 1.0, 2.0],
        vec![f64::INFINITY, 1.0],
        vec![f64::NEG_INFINITY, f64::INFINITY],
        vec![f64::MIN_POSITIVE, f64::MIN_POSITIVE], // denormals-adjacent
        vec![1e308, 1e308],                         // overflow to +inf
    ];
    for &n in &[7usize, 33, 257, 4096] {
        inputs.push((0..n).map(|_| rng.next_f64()).collect());
    }
    inputs
}

/// A candidate is correct if, vs the reference: NaN↦NaN, ±Inf match exactly,
/// otherwise relative error ≤ 1e-9 (abs ≤ 1e-12 near zero).
fn verify(cand: &Candidate, inputs: &[Vec<f64>]) -> (bool, f64) {
    let mut max_rel = 0.0f64;
    for input in inputs {
        let got = (cand.f)(input);
        let want = scalar_ref(input);
        if want.is_nan() {
            if !got.is_nan() {
                return (false, f64::INFINITY);
            }
        } else if want.is_infinite() {
            if got != want {
                return (false, f64::INFINITY);
            }
        } else {
            let denom = want.abs().max(1.0);
            let rel = (got - want).abs() / denom;
            max_rel = max_rel.max(rel);
            if (got - want).abs() > 1e-12 && rel > 1e-9 {
                return (false, rel);
            }
        }
    }
    (true, max_rel)
}

/// Median ns per element over `trials`.
fn measure(cand: &Candidate, data: &[f64], trials: usize) -> f64 {
    let mut samples = Vec::with_capacity(trials);
    for _ in 0..trials {
        let start = Instant::now();
        let r = (cand.f)(data);
        let ns = start.elapsed().as_nanos() as f64;
        std::hint::black_box(r);
        samples.push(ns / data.len().max(1) as f64);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

fn main() {
    let cli = Cli::parse();
    let cands = candidates();
    let corpus = correctness_inputs(cli.seed);
    let scenarios = speed_scenarios(cli.seed);

    // Budget trials so total measurements ≈ --iterations.
    let measured_cands = cands.iter().filter(|c| c.available).count().max(1);
    let trials = (cli.iterations / (measured_cands * scenarios.len()).max(1)).clamp(5, 2000);
    let total_measurements = measured_cands * scenarios.len() * trials;

    // Verify every candidate.
    let verdicts: Vec<(bool, f64)> = cands.iter().map(|c| verify(c, &corpus)).collect();

    // Measure survivors per scenario; ns/elem.
    // results[cand][scenario] = Option<ns_per_elem>
    let mut results: Vec<Vec<Option<f64>>> = Vec::with_capacity(cands.len());
    for (ci, cand) in cands.iter().enumerate() {
        let mut row = Vec::with_capacity(scenarios.len());
        for (_, data) in &scenarios {
            if cand.available && verdicts[ci].0 {
                row.push(Some(measure(cand, data, trials)));
            } else {
                row.push(None);
            }
        }
        results.push(row);
    }

    let report = render_report(
        &cli,
        &cands,
        &verdicts,
        &scenarios,
        &results,
        trials,
        total_measurements,
    );
    std::fs::write(&cli.out, &report).expect("write report");
    // Echo the champion to stdout for the automation script.
    if let Some((name, speedup)) = champion(&cands, &verdicts, &results) {
        println!("champion: {name} ({speedup:.2}x vs scalar_ref on the largest input)");
    }
    println!(
        "wrote {} ({} measurements, {} trials each)",
        cli.out, total_measurements, trials
    );
}

/// The fastest verified candidate on the largest scenario, with its speedup vs
/// the scalar reference.
fn champion(
    cands: &[Candidate],
    verdicts: &[(bool, f64)],
    results: &[Vec<Option<f64>>],
) -> Option<(String, f64)> {
    let last = results.first().map(|r| r.len().saturating_sub(1))?;
    let baseline = results
        .iter()
        .enumerate()
        .find(|(i, _)| cands[*i].name == "scalar_ref")
        .and_then(|(_, r)| r.get(last).copied().flatten())?;
    let mut best: Option<(String, f64)> = None;
    for (i, cand) in cands.iter().enumerate() {
        if !verdicts[i].0 {
            continue;
        }
        if let Some(ns) = results[i].get(last).copied().flatten() {
            let speedup = baseline / ns;
            if best.as_ref().map(|(_, s)| speedup > *s).unwrap_or(true) {
                best = Some((cand.name.to_string(), speedup));
            }
        }
    }
    best
}

fn render_report(
    cli: &Cli,
    cands: &[Candidate],
    verdicts: &[(bool, f64)],
    scenarios: &[(String, Vec<f64>)],
    results: &[Vec<Option<f64>>],
    trials: usize,
    total: usize,
) -> String {
    let mut s = String::new();
    s.push_str("# GlassSheet perf-lab report\n\n");
    s.push_str(&format!(
        "- target: `{}`\n- seed: `{:#x}`\n- trials/scenario: {trials}\n- total measurements: {total}\n",
        std::env::consts::ARCH, cli.seed
    ));
    s.push_str(&format!("- features: {}\n\n", detected_features()));

    s.push_str(
        "## Candidates\n\n| kernel | kind | correctness | max rel.err |\n|---|---|---|---|\n",
    );
    for (i, c) in cands.iter().enumerate() {
        let (ok, rel) = verdicts[i];
        let status = if !c.available {
            "n/a (cpu)".to_string()
        } else if ok {
            "PASS".to_string()
        } else {
            "**FAIL**".to_string()
        };
        let rel_s = if ok && c.available {
            format!("{rel:.1e}")
        } else {
            "—".into()
        };
        s.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            c.name, c.kind, status, rel_s
        ));
    }

    s.push_str("\n## Speed (ns/element, lower is better)\n\n| kernel |");
    for (name, _) in scenarios {
        s.push_str(&format!(" {name} |"));
    }
    s.push_str("\n|---|");
    for _ in scenarios {
        s.push_str("---|");
    }
    s.push('\n');
    for (i, c) in cands.iter().enumerate() {
        s.push_str(&format!("| `{}` |", c.name));
        for cell in &results[i] {
            match cell {
                Some(ns) => s.push_str(&format!(" {ns:.3} |")),
                None => s.push_str(" — |"),
            }
        }
        s.push('\n');
    }

    if let Some((name, speedup)) = champion(cands, verdicts, results) {
        s.push_str(&format!(
            "\n## Champion\n\n`{name}` — **{speedup:.2}×** vs `scalar_ref` on the largest input.\n"
        ));
    }
    s.push_str("\n_Generated by `cargo run -p glasssheet-perf-lab`._\n");
    s
}

fn detected_features() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        let mut f = Vec::new();
        for feat in ["avx", "avx2", "fma", "sse4.2"] {
            if is_x86_feature_detected_str(feat) {
                f.push(feat);
            }
        }
        if f.is_empty() {
            "none detected".into()
        } else {
            f.join(", ")
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        "n/a".into()
    }
}

#[cfg(target_arch = "x86_64")]
fn is_x86_feature_detected_str(feat: &str) -> bool {
    match feat {
        "avx" => std::is_x86_feature_detected!("avx"),
        "avx2" => std::is_x86_feature_detected!("avx2"),
        "fma" => std::is_x86_feature_detected!("fma"),
        "sse4.2" => std::is_x86_feature_detected!("sse4.2"),
        _ => false,
    }
}
