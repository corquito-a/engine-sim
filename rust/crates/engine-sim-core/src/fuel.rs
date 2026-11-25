//! Port of `include/fuel.h` / `src/fuel.cpp`.

use std::sync::Arc;

use crate::constants;
use crate::function::Function;
use crate::units;

/// `Fuel::Parameters`
#[derive(Clone)]
pub struct FuelParameters {
    pub molecular_mass: f64,
    pub energy_density: f64,
    pub density: f64,
    pub turbulence_to_flame_speed_ratio: Arc<Function>,
    pub molecular_afr: f64,
    pub burning_efficiency_randomness: f64,
    pub max_burning_efficiency: f64,
    pub max_dilution_effect: f64,
    pub max_turbulence_effect: f64,
    pub low_efficiency_attenuation: f64,
}

/// `Fuel`
#[derive(Clone)]
pub struct Fuel {
    molecular_mass: f64,
    energy_density: f64,
    density: f64,
    turbulence_to_flame_speed_ratio: Arc<Function>,
    molecular_afr: f64,
    burning_efficiency_randomness: f64,
    max_burning_efficiency: f64,
    max_dilution_effect: f64,
    max_turbulence_effect: f64,
    low_efficiency_attenuation: f64,
}

impl Fuel {
    pub fn new(params: FuelParameters) -> Self {
        Self {
            molecular_mass: params.molecular_mass,
            energy_density: params.energy_density,
            density: params.density,
            turbulence_to_flame_speed_ratio: params.turbulence_to_flame_speed_ratio,
            molecular_afr: params.molecular_afr,
            burning_efficiency_randomness: params.burning_efficiency_randomness,
            max_burning_efficiency: params.max_burning_efficiency,
            max_dilution_effect: params.max_dilution_effect,
            max_turbulence_effect: params.max_turbulence_effect,
            low_efficiency_attenuation: params.low_efficiency_attenuation,
        }
    }

    pub fn molecular_mass(&self) -> f64 {
        self.molecular_mass
    }

    pub fn energy_density(&self) -> f64 {
        self.energy_density
    }

    pub fn density(&self) -> f64 {
        self.density
    }

    pub fn molecular_afr(&self) -> f64 {
        self.molecular_afr
    }

    pub fn burning_efficiency_randomness(&self) -> f64 {
        self.burning_efficiency_randomness
    }

    pub fn max_burning_efficiency(&self) -> f64 {
        self.max_burning_efficiency
    }

    pub fn max_dilution_effect(&self) -> f64 {
        self.max_dilution_effect
    }

    pub fn max_turbulence_effect(&self) -> f64 {
        self.max_turbulence_effect
    }

    /// `Fuel::m_turbulenceToFlameSpeedRatio`
    pub fn turbulence_to_flame_speed_ratio(&self) -> &Function {
        &self.turbulence_to_flame_speed_ratio
    }

    pub fn low_efficiency_attenuation(&self) -> f64 {
        self.low_efficiency_attenuation
    }

    /// `Fuel::flameSpeed`
    pub fn flame_speed(
        &self,
        turbulence: f64,
        molecular_afr: f64,
        t: f64,
        p: f64,
        _firing_pressure: f64,
        _motoring_pressure: f64,
    ) -> f64 {
        let s_l = self.laminar_burning_velocity(molecular_afr, t, p);
        let p_adjustment = 1.0;

        self.turbulence_to_flame_speed_ratio
            .sample_triangle((turbulence / s_l) * p_adjustment)
            * s_l
    }

    /// `Fuel::laminarBurningVelocity`
    pub fn laminar_burning_velocity(&self, molecular_afr: f64, t: f64, p: f64) -> f64 {
        // Assuming fuel is gasoline
        const ER_M: f64 = 1.21;
        let b_m = units::distance(30.5, units::CM) / units::SEC;
        let b_er = -units::distance(54.9, units::CM) / units::SEC;
        let er = molecular_afr / self.molecular_afr;
        let alpha = 2.4 - 0.271 * er.powf(3.51);
        let beta = -0.357 + 0.14 * er.powf(2.77);

        let s_l_0 = b_m + b_er * (er - ER_M) * (er - ER_M);
        let t_ratio = t / units::kelvin(298.0);
        let p_ratio = p / units::pressure(1.0, units::ATM);

        s_l_0 * t_ratio.powf(alpha) * p_ratio.powf(beta)
    }
}

/// Deterministic replacement for the C++ `rand()` used by the combustion
/// efficiency model.
///
/// The original calls the C library's global `rand()`, whose sequence is
/// implementation-defined, so the ignition efficiency (and therefore the exact
/// energy released) is not reproducible across platforms even in C++. This port
/// makes the sequence explicit and seedable; a zero seed reproduces the common
/// MSVC sequence start behaviour only loosely, so parity tests pin the seed on
/// both sides.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

pub const RAND_MAX: f64 = 2147483647.0;

impl Default for Rng {
    fn default() -> Self {
        Self { state: 1 }
    }
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed | 1,
        }
    }

    /// Returns a value in `[0, 1]` equivalent to `rand() / RAND_MAX`.
    pub fn next_unit(&mut self) -> f64 {
        // 31-bit LCG with the same modulus as glibc's `rand()`.
        self.state = self
            .state
            .wrapping_mul(1103515245)
            .wrapping_add(12345)
            & 0x7fff_ffff;
        self.state as f64 / RAND_MAX
    }
}

/// Unused import guard: `constants` is referenced by the C++ original's
/// companion macros in downstream modules.
#[allow(dead_code)]
fn _assert_constants() -> f64 {
    constants::R
}
