//! Port of `include/gas_system.h` / `src/gas_system.cpp`.
//!
//! `GasSystem` is the unit of the simulator's gas dynamics: one lumped control
//! volume holding molecules, internal (kinetic) energy, a composition and a
//! bulk momentum. Every cylinder, runner, plenum and exhaust section in the
//! engine is one of these, and `flow()` moves mass/energy/momentum between
//! them.
//!
//! The port is a statement-for-statement transliteration; see the crate-level
//! fidelity rules. Overloads that C++ resolves implicitly (`initialize` with a
//! default `Mix`, `flow` with a default `Mix`) are presented as an explicit
//! `_default` variant so that call sites stay readable and obviously faithful.

use crate::constants;
use crate::units;
use crate::utilities::clamp;

/// `GasSystem::Mix` — molar fractions of fuel, inert gas and oxygen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub p_fuel: f64,
    pub p_inert: f64,
    pub p_o2: f64,
}

impl Mix {
    pub const fn new(p_fuel: f64, p_inert: f64, p_o2: f64) -> Self {
        Self {
            p_fuel,
            p_inert,
            p_o2,
        }
    }

    /// The C++ default `Mix{}` (member initialisers): pure inert gas.
    pub const DEFAULT: Mix = Mix {
        p_fuel: 0.0,
        p_inert: 1.0,
        p_o2: 0.0,
    };
}

impl Default for Mix {
    fn default() -> Self {
        Mix::DEFAULT
    }
}

/// `GasSystem::State`
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    pub n_mol: f64,
    pub e_k: f64,
    pub volume: f64,
    pub momentum: [f64; 2],
    pub mix: Mix,
}

/// `GasSystem::FlowParameters` minus the two `GasSystem *` members, which become
/// the two `&mut GasSystem` arguments of [`GasSystem::flow`] in Rust.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlowParameters {
    pub k_flow: f64,
    pub dt: f64,
    pub direction_x: f64,
    pub direction_y: f64,
    pub cross_section_area_0: f64,
    pub cross_section_area_1: f64,
}

/// One lumped gas volume.
#[derive(Clone, Debug)]
pub struct GasSystem {
    state: State,

    degrees_of_freedom: i32,

    choked_flow_limit: f64,
    choked_flow_rate_cached: f64,

    width: f64,
    height: f64,
    dx: f64,
    dy: f64,
}

impl Default for GasSystem {
    fn default() -> Self {
        Self {
            state: State::default(),
            degrees_of_freedom: 5,
            choked_flow_limit: 0.0,
            choked_flow_rate_cached: 0.0,
            width: 0.0,
            height: 0.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

impl GasSystem {
    // ---------------------------------------------------------------- geometry

    /// `GasSystem::setGeometry`
    pub fn set_geometry(&mut self, width: f64, height: f64, dx: f64, dy: f64) {
        self.width = width;
        self.height = height;
        self.dx = dx;
        self.dy = dy;
    }

    // -------------------------------------------------------------- lifecycle

    /// `GasSystem::initialize(P, V, T, mix, degreesOfFreedom)`
    pub fn initialize(&mut self, p: f64, v: f64, t: f64, mix: Mix, degrees_of_freedom: i32) {
        self.degrees_of_freedom = degrees_of_freedom;
        self.state.n_mol = p * v / (constants::R * t);
        self.state.volume = v;
        self.state.e_k = t * (0.5 * degrees_of_freedom as f64 * self.state.n_mol * constants::R);
        self.state.mix = mix;
        self.state.momentum[0] = 0.0;
        self.state.momentum[1] = 0.0;

        // The C++ computes `hcr` here and never uses it; omitted.
        self.choked_flow_limit = Self::choked_flow_limit(degrees_of_freedom);
        self.choked_flow_rate_cached = Self::choked_flow_rate(degrees_of_freedom);
    }

    /// `initialize(P, V, T)` with the C++ defaults (`Mix{}`, 5 degrees of freedom).
    pub fn initialize_default(&mut self, p: f64, v: f64, t: f64) {
        self.initialize(p, v, t, Mix::DEFAULT, 5);
    }

    /// `GasSystem::reset(P, T, mix)`
    pub fn reset(&mut self, p: f64, t: f64, mix: Mix) {
        self.state.n_mol = p * self.volume() / (constants::R * t);
        self.state.e_k = t * (0.5 * self.degrees_of_freedom as f64 * self.state.n_mol * constants::R);
        self.state.mix = mix;
        self.state.momentum[0] = 0.0;
        self.state.momentum[1] = 0.0;
    }

    /// `reset(P, T)` with the C++ default `Mix{}`.
    pub fn reset_default(&mut self, p: f64, t: f64) {
        self.reset(p, t, Mix::DEFAULT);
    }

    // ----------------------------------------------------------- state changes

    /// `GasSystem::setVolume`
    pub fn set_volume(&mut self, v: f64) {
        self.change_volume(v - self.state.volume);
    }

    /// `GasSystem::setN`
    pub fn set_n(&mut self, n: f64) {
        self.state.e_k = self.kinetic_energy_at(n);
        self.state.n_mol = n;
    }

    /// `GasSystem::changeVolume`
    pub fn change_volume(&mut self, dv: f64) {
        let v = self.volume();
        let l = (v + dv).powf(1.0 / 3.0);
        let surface_area = l * l;
        let dl = -dv / surface_area;
        let w = dl * self.pressure() * surface_area;

        self.state.volume += dv;
        self.state.e_k += w;
    }

    /// `GasSystem::changePressure`
    pub fn change_pressure(&mut self, dp: f64) {
        self.state.e_k += dp * self.volume() * self.degrees_of_freedom as f64 * 0.5;
    }

    /// `GasSystem::changeTemperature(double dT)`
    pub fn change_temperature(&mut self, dt: f64) {
        self.state.e_k += dt * 0.5 * self.degrees_of_freedom as f64 * self.n() * constants::R;
    }

    /// `GasSystem::changeTemperature(double dT, double n)`
    pub fn change_temperature_n(&mut self, dt: f64, n: f64) {
        self.state.e_k += dt * 0.5 * self.degrees_of_freedom as f64 * n * constants::R;
    }

    /// `GasSystem::changeEnergy`
    pub fn change_energy(&mut self, de: f64) {
        self.state.e_k += de;
    }

    /// `GasSystem::changeMix`
    pub fn change_mix(&mut self, mix: Mix) {
        self.state.mix = mix;
    }

    /// `GasSystem::injectFuel`
    pub fn inject_fuel(&mut self, n: f64) {
        let n_fuel = self.n_fuel() + n;
        let p_fuel = n_fuel / self.n();
        self.state.mix.p_fuel = p_fuel;
    }

    // ---------------------------------------------------------------- reaction

    /// `GasSystem::react`
    ///
    /// Burns up to `n` moles of `mix` against what is in the volume:
    /// `25[O2] + 2[C8H16] -> 16[CO2] + 18[H2O]`.
    /// Returns the moles of fuel actually consumed.
    pub fn react(&mut self, n: f64, mix: Mix) -> f64 {
        let l_n_fuel = mix.p_fuel * n;
        let l_n_o2 = mix.p_o2 * n;

        let system_n_fuel = self.n_fuel();
        let system_n_o2 = self.n_o2();
        let system_n_inert = self.n_inert();
        let system_n = self.n();

        const IDEAL_O2_RATIO: f64 = 25.0 / 2.0;
        const IDEAL_FUEL_RATIO: f64 = 2.0 / 25.0;
        const OUTPUT_INPUT_RATIO: f64 = (16.0 + 18.0) / (25.0 + 2.0);

        let ideal_fuel_n = IDEAL_FUEL_RATIO * l_n_o2;
        let ideal_o2_n = IDEAL_O2_RATIO * l_n_fuel;

        let a_n_fuel = system_n_fuel.min(l_n_fuel).min(ideal_fuel_n);
        let a_n_o2 = system_n_o2.min(l_n_o2).min(ideal_o2_n);

        let reactants_n = a_n_fuel + a_n_o2;
        let products_n = OUTPUT_INPUT_RATIO * reactants_n;
        let dn = products_n - reactants_n;

        self.state.n_mol += dn;

        let new_system_n_fuel = system_n_fuel - a_n_fuel;
        let new_system_n_o2 = system_n_o2 - a_n_o2;
        let new_system_n_inert = system_n_inert + products_n;
        let new_system_n = system_n + dn;

        if new_system_n != 0.0 {
            self.state.mix.p_fuel = new_system_n_fuel / new_system_n;
            self.state.mix.p_inert = new_system_n_inert / new_system_n;
            self.state.mix.p_o2 = new_system_n_o2 / new_system_n;
        } else {
            self.state.mix.p_fuel = 0.0;
            self.state.mix.p_inert = 0.0;
            self.state.mix.p_o2 = 0.0;
        }

        a_n_fuel
    }

    // ------------------------------------------------------------------- flow

    /// `GasSystem::flowConstant`
    pub fn flow_constant(
        target_flow_rate: f64,
        p: f64,
        pressure_drop: f64,
        t: f64,
        hcr: f64,
    ) -> f64 {
        let t_0 = t;
        let p_0 = p;
        let p_t = p - pressure_drop; // p_0 = upstream pressure

        let choked_flow_limit = (2.0 / (hcr + 1.0)).powf(hcr / (hcr - 1.0));
        let p_ratio = p_t / p_0;

        let mut flow_rate;
        if p_ratio <= choked_flow_limit {
            // Choked flow
            flow_rate = hcr.sqrt();
            flow_rate *= (2.0 / (hcr + 1.0)).powf((hcr + 1.0) / (2.0 * (hcr - 1.0)));
        } else {
            flow_rate = (2.0 * hcr) / (hcr - 1.0);
            flow_rate *= 1.0 - p_ratio.powf((hcr - 1.0) / hcr);
            flow_rate = flow_rate.sqrt();
            flow_rate *= p_ratio.powf(1.0 / hcr);
        }

        flow_rate *= p_0 / (constants::R * t_0).sqrt();

        target_flow_rate / flow_rate
    }

    /// `GasSystem::k_28inH2O`
    pub fn k_28in_h2o(flow_rate_scfm: f64) -> f64 {
        Self::flow_constant(
            units::flow(flow_rate_scfm, units::SCFM),
            units::pressure(1.0, units::ATM),
            units::pressure(28.0, units::IN_H2O),
            units::celcius(25.0),
            Self::heat_capacity_ratio_of(5),
        )
    }

    /// `GasSystem::k_carb`
    pub fn k_carb(flow_rate_scfm: f64) -> f64 {
        Self::flow_constant(
            units::flow(flow_rate_scfm, units::SCFM),
            units::pressure(1.0, units::ATM),
            units::pressure(1.5, units::IN_HG),
            units::celcius(25.0),
            Self::heat_capacity_ratio_of(5),
        )
    }

    /// `GasSystem::flowRate` (static)
    #[allow(clippy::too_many_arguments)]
    pub fn flow_rate(
        k_flow: f64,
        p0: f64,
        p1: f64,
        t0: f64,
        t1: f64,
        hcr: f64,
        choked_flow_limit: f64,
        choked_flow_rate_cached: f64,
    ) -> f64 {
        if k_flow == 0.0 {
            return 0.0;
        }

        let direction;
        let t_0;
        let p_0;
        let p_t;
        if p0 > p1 {
            direction = 1.0;
            t_0 = t0;
            p_0 = p0;
            p_t = p1;
        } else {
            direction = -1.0;
            t_0 = t1;
            p_0 = p1;
            p_t = p0;
        }

        let p_ratio = p_t / p_0;
        let mut flow_rate;
        if p_ratio <= choked_flow_limit {
            // Choked flow
            flow_rate = choked_flow_rate_cached;
            flow_rate /= (constants::R * t_0).sqrt();
        } else {
            let s = p_ratio.powf(1.0 / hcr);

            flow_rate = (2.0 * hcr) / (hcr - 1.0);
            flow_rate *= s * (s - p_ratio);
            flow_rate = (flow_rate.max(0.0) / (constants::R * t_0)).sqrt();
        }

        flow_rate *= direction * p_0;

        flow_rate * k_flow
    }

    /// `GasSystem::loseN`
    pub fn lose_n(&mut self, dn: f64, e_k_per_mol: f64) -> f64 {
        self.state.e_k -= e_k_per_mol * dn;
        self.state.n_mol -= dn;

        if self.state.n_mol < 0.0 {
            self.state.n_mol = 0.0;
        }

        dn
    }

    /// `GasSystem::gainN`
    pub fn gain_n(&mut self, dn: f64, e_k_per_mol: f64, mix: Mix) -> f64 {
        let next_n = self.state.n_mol + dn;
        let current_n = self.state.n_mol;

        self.state.e_k += dn * e_k_per_mol;
        self.state.n_mol = next_n;

        if next_n != 0.0 {
            self.state.mix.p_fuel =
                (self.state.mix.p_fuel * current_n + dn * mix.p_fuel) / next_n;
            self.state.mix.p_inert =
                (self.state.mix.p_inert * current_n + dn * mix.p_inert) / next_n;
            self.state.mix.p_o2 = (self.state.mix.p_o2 * current_n + dn * mix.p_o2) / next_n;
        } else {
            self.state.mix.p_fuel = 0.0;
            self.state.mix.p_inert = 0.0;
            self.state.mix.p_o2 = 0.0;
        }

        -dn
    }

    // ------------------------------------------------------------------- bulk

    /// `GasSystem::dissipateExcessVelocity`
    pub fn dissipate_excess_velocity(&mut self) {
        let v_x = self.velocity_x();
        let v_y = self.velocity_y();
        let v_squared = v_x * v_x + v_y * v_y;
        let c = self.c();
        let c_squared = c * c;

        if c_squared >= v_squared || v_squared == 0.0 {
            return;
        }

        let k_squared = c_squared / v_squared;
        let k = k_squared.sqrt();

        self.state.momentum[0] *= k;
        self.state.momentum[1] *= k;

        self.state.e_k += 0.5 * self.mass() * (v_squared - c_squared);

        if self.state.e_k < 0.0 {
            self.state.e_k = 0.0;
        }
    }

    /// `GasSystem::updateVelocity(dt, beta)`
    pub fn update_velocity(&mut self, dt: f64, beta: f64) {
        if self.n() == 0.0 {
            return;
        }

        let depth = self.volume() / (self.width * self.height);

        let mut d_momentum_x = 0.0;
        let mut d_momentum_y = 0.0;

        let p0 = self.dynamic_pressure(self.dx, self.dy);
        let p1 = self.dynamic_pressure(-self.dx, -self.dy);
        let p2 = self.dynamic_pressure(self.dy, self.dx);
        let p3 = self.dynamic_pressure(-self.dy, -self.dx);

        let p_sa_0 = p0 * (self.height * depth);
        let p_sa_1 = p1 * (self.height * depth);
        let p_sa_2 = p2 * (self.width * depth);
        let p_sa_3 = p3 * (self.width * depth);

        d_momentum_x += p_sa_0 * self.dx;
        d_momentum_y += p_sa_0 * self.dy;

        d_momentum_x -= p_sa_1 * self.dx;
        d_momentum_y -= p_sa_1 * self.dy;

        d_momentum_x += p_sa_2 * self.dy;
        d_momentum_y += p_sa_2 * self.dx;

        d_momentum_x -= p_sa_3 * self.dy;
        d_momentum_y -= p_sa_3 * self.dx;

        let m = self.mass();
        let inv_m = 1.0 / m;
        let v0_x = self.state.momentum[0] * inv_m;
        let v0_y = self.state.momentum[1] * inv_m;

        self.state.momentum[0] -= d_momentum_x * dt * beta;
        self.state.momentum[1] -= d_momentum_y * dt * beta;

        let v1_x = self.state.momentum[0] * inv_m;
        let v1_y = self.state.momentum[1] * inv_m;

        self.state.e_k -= 0.5 * m * (v1_x * v1_x - v0_x * v0_x);
        self.state.e_k -= 0.5 * m * (v1_y * v1_y - v0_y * v0_y);

        if self.state.e_k < 0.0 {
            self.state.e_k = 0.0;
        }
    }

    /// `updateVelocity(dt)` with the C++ default `beta = 1.0`.
    pub fn update_velocity_default(&mut self, dt: f64) {
        self.update_velocity(dt, 1.0);
    }

    /// `GasSystem::dissipateVelocity`
    pub fn dissipate_velocity(&mut self, dt: f64, time_constant: f64) {
        if self.n() == 0.0 {
            return;
        }

        let inv_mass = 1.0 / self.mass();
        let velocity_x = self.state.momentum[0] * inv_mass;
        let velocity_y = self.state.momentum[1] * inv_mass;
        let velocity_squared = velocity_x * velocity_x + velocity_y * velocity_y;

        let s = dt / (dt + time_constant);
        self.state.momentum[0] = self.state.momentum[0] * (1.0 - s);
        self.state.momentum[1] = self.state.momentum[1] * (1.0 - s);

        let new_velocity_x = self.state.momentum[0] * inv_mass;
        let new_velocity_y = self.state.momentum[1] * inv_mass;
        let new_velocity_squared = new_velocity_x * new_velocity_x + new_velocity_y * new_velocity_y;

        let d_e_k = 0.5 * self.mass() * (velocity_squared - new_velocity_squared);
        self.state.e_k += d_e_k;
    }

    /// `GasSystem::flow(const FlowParameters &)` — transfer between two volumes.
    ///
    /// `system_0` and `system_1` are the C++ `params.system_0` / `params.system_1`.
    /// Returns the signed flow (positive: from `system_0` to `system_1`).
    pub fn flow(
        system_0: &mut GasSystem,
        system_1: &mut GasSystem,
        params: &FlowParameters,
    ) -> f64 {
        let p_0 = system_0.pressure()
            + system_0.dynamic_pressure(params.direction_x, params.direction_y);
        let p_1 = system_1.pressure()
            + system_1.dynamic_pressure(-params.direction_x, -params.direction_y);

        let dx;
        let dy;
        let direction;
        let source_pressure;
        let sink_pressure;
        let source_cross_section;
        let sink_cross_section;

        // Split the borrow so that `source` and `sink` are disjoint `&mut`s,
        // exactly mirroring the C++ pointer swap.
        let (source, sink): (&mut GasSystem, &mut GasSystem) = if p_0 > p_1 {
            dx = params.direction_x;
            dy = params.direction_y;
            direction = 1.0;
            source_pressure = p_0;
            sink_pressure = p_1;
            source_cross_section = params.cross_section_area_0;
            sink_cross_section = params.cross_section_area_1;
            (&mut *system_0, &mut *system_1)
        } else {
            dx = -params.direction_x;
            dy = -params.direction_y;
            direction = -1.0;
            source_pressure = p_1;
            sink_pressure = p_0;
            source_cross_section = params.cross_section_area_1;
            sink_cross_section = params.cross_section_area_0;
            (&mut *system_1, &mut *system_0)
        };

        let mut flow = params.dt
            * Self::flow_rate(
                params.k_flow,
                source_pressure,
                sink_pressure,
                source.temperature(),
                sink.temperature(),
                source.heat_capacity_ratio(),
                source.choked_flow_limit,
                source.choked_flow_rate_cached,
            );

        // The C++ also evaluates `source->pressureEquilibriumMaxFlow(sink)` into
        // an unused local here; omitted (no side effects).
        flow = clamp(flow, 0.0, 0.9 * source.n());

        let fraction = flow / source.n();
        let fraction_volume = fraction * source.volume();
        let fraction_mass = fraction * source.mass();

        if flow != 0.0 {
            // - Stage 1
            // A fraction flows from source to sink.
            let e_k_bulk_src0 = source.bulk_kinetic_energy();
            let e_k_bulk_sink0 = sink.bulk_kinetic_energy();

            let e_k_per_mol = source.kinetic_energy_per_mol();
            sink.gain_n(flow, e_k_per_mol, source.mix());
            source.lose_n(flow, e_k_per_mol);

            // (`s0`/`s1` energy sums are computed and unused in the C++.)
            let dp_x = source.state.momentum[0] * fraction;
            let dp_y = source.state.momentum[1] * fraction;
            source.state.momentum[0] -= dp_x;
            source.state.momentum[1] -= dp_y;

            sink.state.momentum[0] += dp_x;
            sink.state.momentum[1] += dp_y;

            let e_k_bulk_src1 = source.bulk_kinetic_energy();
            let e_k_bulk_sink1 = sink.bulk_kinetic_energy();

            sink.state.e_k -=
                (e_k_bulk_src1 + e_k_bulk_sink1) - (e_k_bulk_src0 + e_k_bulk_sink0);
        }

        let source_mass = source.mass();
        let inv_source_mass = 1.0 / source_mass;
        let sink_mass = sink.mass();
        let inv_sink_mass = 1.0 / sink_mass;

        let c_source = source.c();
        let c_sink = sink.c();

        let source_initial_momentum_x = source.state.momentum[0];
        let source_initial_momentum_y = source.state.momentum[1];

        let sink_initial_momentum_x = sink.state.momentum[0];
        let sink_initial_momentum_y = sink.state.momentum[1];

        // Momentum in the transferred fraction.
        if sink_cross_section != 0.0 {
            let sink_fraction_velocity =
                clamp((fraction_volume / sink_cross_section) / params.dt, 0.0, c_sink);
            let sink_fraction_velocity_x = sink_fraction_velocity * dx;
            let sink_fraction_velocity_y = sink_fraction_velocity * dy;
            let sink_fraction_momentum_x = sink_fraction_velocity_x * fraction_mass;
            let sink_fraction_momentum_y = sink_fraction_velocity_y * fraction_mass;

            sink.state.momentum[0] += sink_fraction_momentum_x;
            sink.state.momentum[1] += sink_fraction_momentum_y;
        }

        if source_cross_section != 0.0 && source_mass != 0.0 {
            let source_fraction_velocity = clamp(
                (fraction_volume / source_cross_section) / params.dt,
                0.0,
                c_source,
            );
            let source_fraction_velocity_x = source_fraction_velocity * dx;
            let source_fraction_velocity_y = source_fraction_velocity * dy;
            let source_fraction_momentum_x = source_fraction_velocity_x * fraction_mass;
            let source_fraction_momentum_y = source_fraction_velocity_y * fraction_mass;

            source.state.momentum[0] += source_fraction_momentum_x;
            source.state.momentum[1] += source_fraction_momentum_y;
        }

        if source_mass != 0.0 {
            // Energy conservation
            let source_velocity0_x = source_initial_momentum_x * inv_source_mass;
            let source_velocity0_y = source_initial_momentum_y * inv_source_mass;

            let source_velocity1_x = source.state.momentum[0] * inv_source_mass;
            let source_velocity1_y = source.state.momentum[1] * inv_source_mass;

            source.state.e_k -= 0.5
                * source_mass
                * (source_velocity1_x * source_velocity1_x
                    - source_velocity0_x * source_velocity0_x);

            source.state.e_k -= 0.5
                * source_mass
                * (source_velocity1_y * source_velocity1_y
                    - source_velocity0_y * source_velocity0_y);
        }

        if sink_mass > 0.0 {
            let sink_velocity0_x = sink_initial_momentum_x * inv_sink_mass;
            let sink_velocity0_y = sink_initial_momentum_y * inv_sink_mass;

            let sink_velocity1_x = sink.state.momentum[0] * inv_sink_mass;
            let sink_velocity1_y = sink.state.momentum[1] * inv_sink_mass;

            sink.state.e_k -= 0.5
                * sink_mass
                * (sink_velocity1_x * sink_velocity1_x - sink_velocity0_x * sink_velocity0_x);

            sink.state.e_k -= 0.5
                * sink_mass
                * (sink_velocity1_y * sink_velocity1_y - sink_velocity0_y * sink_velocity0_y);
        }

        if sink.state.e_k < 0.0 {
            sink.state.e_k = 0.0;
        }

        if source.state.e_k < 0.0 {
            source.state.e_k = 0.0;
        }

        flow * direction
    }

    /// `GasSystem::flow(k_flow, dt, P_env, T_env, mix)` — flow to/from the
    /// infinite environment.
    pub fn flow_env(&mut self, k_flow: f64, dt: f64, p_env: f64, t_env: f64, mix: Mix) -> f64 {
        let max_flow = self.pressure_equilibrium_max_flow_env(p_env, t_env);
        let mut flow = dt
            * Self::flow_rate(
                k_flow,
                self.pressure(),
                p_env,
                self.temperature(),
                t_env,
                self.heat_capacity_ratio(),
                self.choked_flow_limit,
                self.choked_flow_rate_cached,
            );

        if flow.abs() > max_flow.abs() {
            flow = max_flow;
        }

        if flow < 0.0 {
            let bulk_e_k_0 = self.bulk_kinetic_energy();
            let e_k_per_mol = Self::kinetic_energy_per_mol_t(t_env, self.degrees_of_freedom);
            self.gain_n(-flow, e_k_per_mol, mix);
            let bulk_e_k_1 = self.bulk_kinetic_energy();

            self.state.e_k += bulk_e_k_1 - bulk_e_k_0;
        } else {
            let starting_n = self.n();
            let e_k_per_mol = self.kinetic_energy_per_mol();
            self.lose_n(flow, e_k_per_mol);

            self.state.momentum[0] -= (flow / starting_n) * self.state.momentum[0];
            self.state.momentum[1] -= (flow / starting_n) * self.state.momentum[1];
        }

        flow
    }

    /// `flow(k_flow, dt, P_env, T_env)` with the C++ default `Mix{}`.
    pub fn flow_env_default(&mut self, k_flow: f64, dt: f64, p_env: f64, t_env: f64) -> f64 {
        self.flow_env(k_flow, dt, p_env, t_env, Mix::DEFAULT)
    }

    /// `GasSystem::pressureEquilibriumMaxFlow(const GasSystem *b)`
    pub fn pressure_equilibrium_max_flow(&self, b: &GasSystem) -> f64 {
        if self.pressure() > b.pressure() {
            let max_flow = (b.volume() * self.kinetic_energy() - self.volume() * b.kinetic_energy())
                / (b.volume() * self.kinetic_energy_per_mol()
                    + self.volume() * self.kinetic_energy_per_mol());
            max_flow.max(0.0).min(self.n())
        } else {
            let max_flow = (b.volume() * self.kinetic_energy() - self.volume() * b.kinetic_energy())
                / (b.volume() * b.kinetic_energy_per_mol()
                    + self.volume() * b.kinetic_energy_per_mol());
            max_flow.min(0.0).max(-b.n())
        }
    }

    /// `GasSystem::pressureEquilibriumMaxFlow(double P_env, double T_env)`
    pub fn pressure_equilibrium_max_flow_env(&self, p_env: f64, t_env: f64) -> f64 {
        if self.pressure() > p_env {
            -(p_env * (0.5 * self.degrees_of_freedom as f64 * self.volume())
                - self.kinetic_energy())
                / self.kinetic_energy_per_mol()
        } else {
            let e_k_per_mol_env = 0.5 * t_env * constants::R * self.degrees_of_freedom as f64;
            -(p_env * (0.5 * self.degrees_of_freedom as f64 * self.volume())
                - self.kinetic_energy())
                / e_k_per_mol_env
        }
    }

    // -------------------------------------------------------------- accessors

    /// `GasSystem::kineticEnergyPerMol(double T, int degreesOfFreedom)` (static)
    pub const fn kinetic_energy_per_mol_t(t: f64, degrees_of_freedom: i32) -> f64 {
        0.5 * t * constants::R * degrees_of_freedom as f64
    }

    /// `GasSystem::kineticEnergyPerMol()`
    pub fn kinetic_energy_per_mol(&self) -> f64 {
        self.kinetic_energy_at(1.0)
    }

    /// `GasSystem::heatCapacityRatio(int degreesOfFreedom)` (static)
    pub const fn heat_capacity_ratio_of(degrees_of_freedom: i32) -> f64 {
        1.0 + (2.0 / degrees_of_freedom as f64)
    }

    /// `GasSystem::chokedFlowLimit(int degreesOfFreedom)` (static)
    pub fn choked_flow_limit(degrees_of_freedom: i32) -> f64 {
        let hcr = Self::heat_capacity_ratio_of(degrees_of_freedom);
        (2.0 / (hcr + 1.0)).powf(hcr / (hcr - 1.0))
    }

    /// `GasSystem::chokedFlowRate(int degreesOfFreedom)` (static)
    pub fn choked_flow_rate(degrees_of_freedom: i32) -> f64 {
        let hcr = Self::heat_capacity_ratio_of(degrees_of_freedom);
        hcr.sqrt() * (2.0 / (hcr + 1.0)).powf((hcr + 1.0) / (2.0 * (hcr - 1.0)))
    }

    /// `GasSystem::approximateDensity`
    pub fn approximate_density(&self) -> f64 {
        (units::AIR_MOLECULAR_MASS * self.n()) / self.volume()
    }

    /// `GasSystem::n()`
    pub fn n(&self) -> f64 {
        self.state.n_mol
    }

    /// `GasSystem::n(double V)`
    pub fn n_at_volume(&self, v: f64) -> f64 {
        (v / self.volume()) * self.n()
    }

    /// `GasSystem::kineticEnergy()`
    pub fn kinetic_energy(&self) -> f64 {
        self.state.e_k
    }

    /// `GasSystem::kineticEnergy(double n)`
    pub fn kinetic_energy_at(&self, n: f64) -> f64 {
        (self.kinetic_energy() / self.n()) * n
    }

    /// `GasSystem::c()` — speed of sound
    pub fn c(&self) -> f64 {
        if self.n() == 0.0 || self.kinetic_energy() == 0.0 {
            return 0.0;
        }

        let hcr = self.heat_capacity_ratio();
        let static_pressure = self.pressure();
        let density = self.approximate_density();
        (static_pressure * hcr / density).sqrt()
    }

    /// `GasSystem::totalEnergy()`
    pub fn total_energy(&self) -> f64 {
        if self.n() == 0.0 {
            return 0.0;
        }

        let inv_mass = 1.0 / self.mass();
        let v_x = self.state.momentum[0] * inv_mass;
        let v_y = self.state.momentum[1] * inv_mass;
        let v_squared = v_x * v_x + v_y * v_y;

        self.kinetic_energy() + 0.5 * self.mass() * v_squared
    }

    /// `GasSystem::bulkKineticEnergy()`
    pub fn bulk_kinetic_energy(&self) -> f64 {
        let m = self.mass();
        if m == 0.0 {
            return 0.0;
        }

        let v_x = self.state.momentum[0] / m;
        let v_y = self.state.momentum[1] / m;
        let v_squared = v_x * v_x + v_y * v_y;
        0.5 * m * v_squared
    }

    /// `GasSystem::dynamicPressure(dx, dy)`
    pub fn dynamic_pressure(&self, dx: f64, dy: f64) -> f64 {
        if self.n() == 0.0 || self.kinetic_energy() == 0.0 {
            return 0.0;
        }

        let inverse_mass = 1.0 / self.mass();
        let v = inverse_mass * (dx * self.state.momentum[0] + dy * self.state.momentum[1]);

        if v <= 0.0 {
            return 0.0;
        }

        let hcr = self.heat_capacity_ratio();
        let static_pressure = self.pressure();
        let density = self.approximate_density();
        let c_squared = static_pressure * hcr / density;
        let mach_number_squared = v * v / c_squared;

        // Equivalent to:
        // staticPressure * pow(1 + ((hcr - 1) / 2) * M^2, hcr / (hcr - 1)) - 1)
        let x = 1.0 + ((hcr - 1.0) / 2.0) * mach_number_squared;
        let x_d;
        match self.degrees_of_freedom {
            3 => x_d = x * x * x * x * x,
            5 => {
                let x_2 = x * x;
                let x_3 = x_2 * x;
                x_d = x_3 * x_3 * x;
            }
            _ => x_d = x,
        };

        static_pressure * (x_d.sqrt() - 1.0)
    }

    /// `GasSystem::mass()`
    pub fn mass(&self) -> f64 {
        units::AIR_MOLECULAR_MASS * self.n()
    }

    /// `GasSystem::pressure()`
    pub fn pressure(&self) -> f64 {
        let volume = self.volume();
        if volume != 0.0 {
            self.kinetic_energy() / (0.5 * self.degrees_of_freedom as f64 * volume)
        } else {
            0.0
        }
    }

    /// `GasSystem::temperature()`
    pub fn temperature(&self) -> f64 {
        if self.n() == 0.0 {
            0.0
        } else {
            self.kinetic_energy() / (0.5 * self.degrees_of_freedom as f64 * self.n() * constants::R)
        }
    }

    /// `GasSystem::velocity_x()`
    pub fn velocity_x(&self) -> f64 {
        if self.n() == 0.0 {
            0.0
        } else {
            self.state.momentum[0] / self.mass()
        }
    }

    /// `GasSystem::velocity_y()`
    pub fn velocity_y(&self) -> f64 {
        if self.n() == 0.0 {
            0.0
        } else {
            self.state.momentum[1] / self.mass()
        }
    }

    /// `GasSystem::volume()`
    pub fn volume(&self) -> f64 {
        self.state.volume
    }

    /// `GasSystem::volume(double n)`
    pub fn volume_at(&self, n: f64) -> f64 {
        n * self.n() / self.volume()
    }

    /// `GasSystem::n_fuel()`
    pub fn n_fuel(&self) -> f64 {
        self.state.mix.p_fuel * self.n()
    }

    /// `GasSystem::n_inert()`
    pub fn n_inert(&self) -> f64 {
        self.state.mix.p_inert * self.n()
    }

    /// `GasSystem::n_o2()`
    pub fn n_o2(&self) -> f64 {
        self.state.mix.p_o2 * self.n()
    }

    /// `GasSystem::mix()`
    pub fn mix(&self) -> Mix {
        self.state.mix
    }

    /// `GasSystem::heatCapacityRatio()`
    pub fn heat_capacity_ratio(&self) -> f64 {
        Self::heat_capacity_ratio_of(self.degrees_of_freedom)
    }

    /// `GasSystem::degreesOfFreedom()`
    pub fn degrees_of_freedom(&self) -> i32 {
        self.degrees_of_freedom
    }

    /// Direct read of the internal state (used by tests/diagnostics).
    pub fn state(&self) -> &State {
        &self.state
    }
}
