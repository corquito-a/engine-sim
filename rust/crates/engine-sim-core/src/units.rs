//! Port of `include/units.h`.
//!
//! The original is a set of `constexpr` scaling factors plus trivial
//! `constexpr` conversions. They are dimensionless multipliers applied to
//! `double`, so the port is a straight 1:1 mapping into `f64` constants and
//! `const fn`s. Keeping them `const` (not `static`) means they are usable in
//! const contexts exactly like the C++ originals.

use crate::constants;

// ---------------------------------------------------------------- force
pub const N: f64 = 1.0;
pub const LBF: f64 = N * 4.44822;

// ---------------------------------------------------------------- mass
pub const KG: f64 = 1.0;
pub const G: f64 = KG / 1000.0;
pub const LB: f64 = 0.45359237 * KG;

// ---------------------------------------------------------------- distance
pub const M: f64 = 1.0;
pub const CM: f64 = M / 100.0;
pub const MM: f64 = M / 1000.0;
pub const KM: f64 = M * 1000.0;
pub const INCH: f64 = CM * 2.54;
pub const FOOT: f64 = INCH * 12.0;
pub const THOU: f64 = INCH / 1000.0;
pub const MILE: f64 = M * 1609.344;

// ---------------------------------------------------------------- time
pub const SEC: f64 = 1.0;
pub const MINUTE: f64 = 60.0 * SEC;
pub const HOUR: f64 = 60.0 * MINUTE;

// ---------------------------------------------------------------- torque
pub const NM: f64 = N * M;
pub const FT_LB: f64 = FOOT * LBF;

// ---------------------------------------------------------------- power
pub const W: f64 = NM / SEC;
pub const KW: f64 = W * 1000.0;
pub const HP: f64 = 745.699872 * W;

// ---------------------------------------------------------------- volume
pub const M3: f64 = 1.0;
pub const CC: f64 = CM * CM * CM;
pub const ML: f64 = CC;
pub const L: f64 = ML * 1000.0;
pub const CUBIC_FEET: f64 = FOOT * FOOT * FOOT;
pub const CUBIC_INCHES: f64 = INCH * INCH * INCH;
pub const GAL: f64 = 3.785411784 * L;

// ---------------------------------------------------------------- molecular
pub const MOL: f64 = 1.0;
pub const KMOL: f64 = MOL / 1000.0;
pub const MMOL: f64 = MOL / 1_000_000.0;
pub const LBMOL: f64 = MOL * 453.59237;

// ---------------------------------------------------------------- flow rate (moles)
pub const MOL_PER_SEC: f64 = MOL / SEC;
pub const SCFM: f64 = 0.002641 * LBMOL / MINUTE;

// ---------------------------------------------------------------- area
pub const M2: f64 = 1.0;
pub const CM2: f64 = CM * CM;

// ---------------------------------------------------------------- pressure
pub const PA: f64 = 1.0;
pub const KPA: f64 = PA * 1000.0;
pub const MPA: f64 = PA * 1_000_000.0;
pub const ATM: f64 = 101.325 * KPA;
pub const MBAR: f64 = PA * 100.0;
pub const BAR: f64 = MBAR * 1000.0;
pub const PSI: f64 = LBF / (INCH * INCH);
pub const PSIG: f64 = PSI;
pub const IN_HG: f64 = PA * 3386.3886666666713;
pub const IN_H2O: f64 = IN_HG * 0.0734824;

// ---------------------------------------------------------------- temperature
pub const K: f64 = 1.0;
pub const K0: f64 = 273.15;
pub const C: f64 = K;
pub const F: f64 = (5.0 / 9.0) * K;
pub const F0: f64 = -459.67;

// ---------------------------------------------------------------- energy
pub const J: f64 = 1.0;
pub const KJ: f64 = J * 1000.0;
pub const MJ: f64 = J * 1_000_000.0;

// ---------------------------------------------------------------- angles
pub const RAD: f64 = 1.0;
pub const DEG: f64 = RAD * (constants::PI / 180.0);

/// `units::AirMolecularMass` — kg/mol.
pub const AIR_MOLECULAR_MASS: f64 = mass(28.97, G) / MOL;

// ------------------------------------------------------------ conversions

pub const fn distance(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn area(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn torque(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn rpm(rpm: f64) -> f64 {
    rpm * 0.104719755
}

pub const fn to_rpm(rad_s: f64) -> f64 {
    rad_s / 0.104719755
}

pub const fn pressure(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn psia(p: f64) -> f64 {
    pressure(p, PSIG) - pressure(1.0, ATM)
}

pub const fn to_psia(p: f64) -> f64 {
    (p + pressure(1.0, ATM)) / PSIG
}

pub const fn mass(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn force(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn volume(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn flow(v: f64, unit: f64) -> f64 {
    v * unit
}

/// `units::convert(v, unit0, unit1)`
pub const fn convert_units(v: f64, unit0: f64, unit1: f64) -> f64 {
    v * (unit0 / unit1)
}

/// `units::convert(v, unit)`
pub const fn convert(v: f64, unit: f64) -> f64 {
    v / unit
}

pub const fn celcius(t_c: f64) -> f64 {
    t_c * C + K0
}

pub const fn kelvin(t: f64) -> f64 {
    t * K
}

pub const fn fahrenheit(t_f: f64) -> f64 {
    F * (t_f - F0)
}

pub const fn to_absolute_fahrenheit(t: f64) -> f64 {
    t / F
}

pub const fn angle(v: f64, unit: f64) -> f64 {
    v * unit
}

pub const fn energy(v: f64, unit: f64) -> f64 {
    v * unit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_factors_match_reference() {
        // values cross-checked against the C++ constants
        assert_eq!(ATM, 101_325.0);
        assert_eq!(AIR_MOLECULAR_MASS, 0.02897);
        assert_eq!(celcius(25.0), 298.15);
        // `convert(v, unit)` expresses v *in* that unit.
        assert!((convert(ATM, ATM) - 1.0).abs() < 1e-15);
        assert!((convert(1.0, ATM) - 1.0 / 101_325.0).abs() < 1e-20);
    }
}
