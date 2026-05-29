//! Numeric reduction kernels behind the aggregate functions.
//!
//! These are kept as a small, allocation-free API (`&[f64] -> f64`) for two
//! reasons: (1) the chunked implementations autovectorize to SIMD on modern
//! targets, and (2) they are the stable reference the autotuner
//! (`crates/perf-lab`) benchmarks alternative implementations against. The
//! chunked `sum`/`product` reorder additions into independent accumulators, so
//! their rounding can differ from a strict left-to-right fold by a few ULP —
//! acceptable for a spreadsheet and well within the autotuner's tolerance gate.

/// Sum with four independent accumulators (autovectorizes to `vaddpd`).
pub fn sum(xs: &[f64]) -> f64 {
    let mut acc = [0.0f64; 4];
    let mut chunks = xs.chunks_exact(4);
    for c in &mut chunks {
        acc[0] += c[0];
        acc[1] += c[1];
        acc[2] += c[2];
        acc[3] += c[3];
    }
    let mut total = (acc[0] + acc[1]) + (acc[2] + acc[3]);
    for &x in chunks.remainder() {
        total += x;
    }
    total
}

/// Product with four independent accumulators.
pub fn product(xs: &[f64]) -> f64 {
    let mut acc = [1.0f64; 4];
    let mut chunks = xs.chunks_exact(4);
    for c in &mut chunks {
        acc[0] *= c[0];
        acc[1] *= c[1];
        acc[2] *= c[2];
        acc[3] *= c[3];
    }
    let mut total = (acc[0] * acc[1]) * (acc[2] * acc[3]);
    for &x in chunks.remainder() {
        total *= x;
    }
    total
}

/// Minimum, NaN-ignoring (`f64::min` semantics). `None` for an empty slice.
pub fn min(xs: &[f64]) -> Option<f64> {
    xs.iter().copied().reduce(f64::min)
}

/// Maximum, NaN-ignoring. `None` for an empty slice.
pub fn max(xs: &[f64]) -> Option<f64> {
    xs.iter().copied().reduce(f64::max)
}

/// Strict left-to-right sum — the reference the autotuner and tests compare
/// against.
pub fn sum_ref(xs: &[f64]) -> f64 {
    let mut s = 0.0;
    for &x in xs {
        s += x;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_matches_reference_within_tolerance() {
        let xs: Vec<f64> = (1..=1000).map(|i| i as f64 * 0.1).collect();
        let fast = sum(&xs);
        let reference = sum_ref(&xs);
        assert!((fast - reference).abs() <= reference.abs() * 1e-12);
    }

    #[test]
    fn handles_short_and_empty() {
        assert_eq!(sum(&[]), 0.0);
        assert_eq!(sum(&[5.0]), 5.0);
        assert_eq!(sum(&[1.0, 2.0, 3.0]), 6.0);
        assert_eq!(product(&[2.0, 3.0, 4.0]), 24.0);
        assert_eq!(min(&[3.0, 1.0, 2.0]), Some(1.0));
        assert_eq!(max(&[3.0, 1.0, 2.0]), Some(3.0));
        assert_eq!(min(&[]), None);
    }

    #[test]
    fn nan_and_inf_behaviour() {
        assert!(sum(&[1.0, f64::NAN, 2.0]).is_nan());
        assert_eq!(sum(&[1.0, f64::INFINITY]), f64::INFINITY);
        // f64::min ignores NaN.
        assert_eq!(min(&[f64::NAN, 1.0, 2.0]), Some(1.0));
    }
}
