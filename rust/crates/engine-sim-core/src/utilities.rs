//! Port of `include/utilities.h` / `src/utilities.cpp`.

/// `clamp(x, x0, x1)` — the C++ template, including its exact comparison
/// order (which matters for signed zeros and, historically, for NaN).
#[inline]
pub fn clamp<T: PartialOrd>(x: T, x0: T, x1: T) -> T {
    if x <= x0 {
        x0
    } else if x >= x1 {
        x1
    } else {
        x
    }
}

/// `clamp(x)` with the C++ default arguments `[0, 1]`.
#[inline]
pub fn clamp01<T: PartialOrd + From<u8>>(x: T) -> T {
    clamp(x, T::from(0), T::from(1))
}

/// `modularDistance(a, b, mod)`
pub fn modular_distance(a0: f64, b0: f64, modulus: f64) -> f64 {
    let (a, b) = if a0 < b0 { (a0, b0) } else { (b0, a0) };
    (b - a).min(a + modulus - b)
}

/// `positiveMod(x, mod)`
pub fn positive_mod(x0: f64, modulus: f64) -> f64 {
    let mut x = x0;
    if x < 0.0 {
        x = (-x / modulus).ceil() * modulus + x;
    }
    x % modulus
}

/// `erfApproximation(x)` — Abramowitz & Stegun 7.1.26 style rational fit.
pub fn erf_approximation(x: f64) -> f64 {
    const A1: f64 = 0.278393;
    const A2: f64 = 0.230389;
    const A3: f64 = 0.000972;
    const A4: f64 = 0.078108;

    let x2 = x * x;
    let x3 = x2 * x;
    let x4 = x3 * x;

    let q = 1.0 / (1.0 + A1 * x + A2 * x2 + A3 * x3 + A4 * x4);
    let q2 = q * q;
    let q4 = q2 * q2;

    1.0 - q4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_matches_reference_semantics() {
        assert_eq!(clamp(-1.0, 0.0, 1.0), 0.0);
        assert_eq!(clamp(2.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp(0.5, 0.0, 1.0), 0.5);
        assert_eq!(clamp01(0.25), 0.25);
    }

    #[test]
    fn modular_distance_is_symmetric() {
        assert!((modular_distance(0.9, 0.1, 1.0) - 0.2).abs() < 1e-12);
        assert_eq!(modular_distance(0.1, 0.9, 1.0), modular_distance(0.9, 0.1, 1.0));
    }

    #[test]
    fn positive_mod_is_positive() {
        assert!(positive_mod(-0.25, 1.0) >= 0.0);
        assert!((positive_mod(-0.25, 1.0) - 0.75).abs() < 1e-12);
    }
}
