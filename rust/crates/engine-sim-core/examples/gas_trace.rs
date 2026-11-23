//! Parity trace generator (Rust side).
//!
//! Runs exactly the same scenario as `rust/tools/parity/gas_reference.cpp`
//! against the ported `GasSystem` and prints every checkpoint as raw IEEE-754
//! bits, so `run_parity.sh` can diff the two outputs and prove bit-exact
//! agreement with the original C++ implementation.
//!
//! Run: `cargo run --release --example gas_trace`

use engine_sim_core::gas::{FlowParameters, GasSystem, Mix};
use engine_sim_core::units;

fn emit(name: &str, v: f64) {
    println!("{name},{:016x}", v.to_bits());
}

fn main() {
    // ------------------------------------------------------------------
    // A. Single-volume thermodynamics
    // ------------------------------------------------------------------
    {
        let mut a = GasSystem::default();
        a.initialize_default(
            units::pressure(1.0, units::ATM),
            units::volume(1.0, units::CC),
            units::celcius(25.0),
        );

        emit("A.n", a.n());
        emit("A.P", a.pressure());
        emit("A.T", a.temperature());
        emit("A.Ek", a.kinetic_energy());
        emit("A.EkPerMol", a.kinetic_energy_per_mol());
        emit("A.mass", a.mass());
        emit("A.rho", a.approximate_density());
        emit("A.c", a.c());
        emit("A.hcr", a.heat_capacity_ratio());
        emit("A.totalE", a.total_energy());
        emit("A.bulkEk", a.bulk_kinetic_energy());
        emit("A.vx", a.velocity_x());
        emit("A.vy", a.velocity_y());
        emit("A.dp.1.0", a.dynamic_pressure(1.0, 0.0));
        emit("A.chokedLimit", GasSystem::choked_flow_limit(5));
        emit("A.chokedRate", GasSystem::choked_flow_rate(5));
        emit("A.k28", GasSystem::k_28in_h2o(300.0));
        emit("A.kcarb", GasSystem::k_carb(500.0));
        emit(
            "A.flowConstant",
            GasSystem::flow_constant(
                units::flow(400.0, units::SCFM),
                units::pressure(2.5, units::ATM),
                units::pressure(1.5, units::ATM),
                units::celcius(2000.0),
                GasSystem::heat_capacity_ratio_of(5),
            ),
        );
        emit(
            "A.flowRate.choked",
            GasSystem::flow_rate(
                GasSystem::k_carb(400.0),
                units::pressure(2.5, units::ATM),
                units::pressure(1.0, units::ATM),
                units::celcius(2000.0),
                units::celcius(25.0),
                GasSystem::heat_capacity_ratio_of(5),
                GasSystem::choked_flow_limit(5),
                GasSystem::choked_flow_rate(5),
            ),
        );
        emit(
            "A.flowRate.subcritical",
            GasSystem::flow_rate(
                GasSystem::k_carb(400.0),
                units::pressure(2.5, units::ATM),
                units::pressure(2.0, units::ATM),
                units::celcius(2000.0),
                units::celcius(25.0),
                GasSystem::heat_capacity_ratio_of(5),
                GasSystem::choked_flow_limit(5),
                GasSystem::choked_flow_rate(5),
            ),
        );
        emit(
            "A.flowRate.zeroK",
            GasSystem::flow_rate(
                0.0,
                units::pressure(2.5, units::ATM),
                units::pressure(2.0, units::ATM),
                units::celcius(2000.0),
                units::celcius(25.0),
                GasSystem::heat_capacity_ratio_of(5),
                GasSystem::choked_flow_limit(5),
                GasSystem::choked_flow_rate(5),
            ),
        );

        // Compression stroke (100 steps, emit every 10th).
        let dv = units::volume(0.5, units::CC);
        for i in 0..100 {
            a.change_volume(-dv / 100.0);
            if i % 10 == 9 {
                emit("A.comp.P", a.pressure());
                emit("A.comp.T", a.temperature());
                emit("A.comp.Ek", a.kinetic_energy());
            }
        }

        a.change_pressure(units::pressure(0.5, units::ATM));
        emit("A.chgP.P", a.pressure());
        a.change_temperature(50.0);
        emit("A.chgT.T", a.temperature());
        let n = a.n() * 0.1;
        a.change_temperature_n(10.0, n);
        emit("A.chgTN.T", a.temperature());
        a.change_energy(0.25);
        emit("A.chgE.Ek", a.kinetic_energy());

        let fuel = a.n() * 0.02;
        a.inject_fuel(fuel);
        let mix = a.mix();
        emit("A.inject.pFuel", mix.p_fuel);
        emit("A.inject.nFuel", a.n_fuel());

        let charge = a.n() * 0.5;
        let reacted = a.react(charge, Mix::new(0.0, 0.79, 0.21));
        emit("A.react.return", reacted);
        emit("A.react.n", a.n());
        let mix = a.mix();
        emit("A.react.pFuel", mix.p_fuel);
        emit("A.react.pInert", mix.p_inert);
        emit("A.react.pO2", mix.p_o2);
        emit("A.react.P", a.pressure());
        emit("A.react.T", a.temperature());

        let e_k_per_mol = a.kinetic_energy_per_mol();
        let gain = a.n() * 0.3;
        a.gain_n(gain, e_k_per_mol, Mix::new(0.1, 0.8, 0.1));
        emit("A.gain.n", a.n());
        let mix = a.mix();
        emit("A.gain.pFuel", mix.p_fuel);
        emit("A.gain.pInert", mix.p_inert);
        emit("A.gain.pO2", mix.p_o2);

        let e_k_per_mol_2 = a.kinetic_energy_per_mol();
        let loss = a.n() * 0.2;
        a.lose_n(loss, e_k_per_mol_2);
        emit("A.lose.n", a.n());
        emit("A.lose.T", a.temperature());

        emit(
            "A.maxFlow.out",
            a.pressure_equilibrium_max_flow_env(
                units::pressure(1.0, units::ATM),
                units::celcius(25.0),
            ),
        );
        emit(
            "A.maxFlow.in",
            a.pressure_equilibrium_max_flow_env(
                units::pressure(6.0, units::ATM),
                units::celcius(25.0),
            ),
        );

        for i in 0..100 {
            let f = a.flow_env_default(
                GasSystem::k_28in_h2o(200.0),
                1.0 / 10000.0,
                units::pressure(1.0, units::ATM),
                units::celcius(25.0),
            );
            if i % 20 == 19 {
                emit("A.env.flow", f);
                emit("A.env.P", a.pressure());
                emit("A.env.T", a.temperature());
                emit("A.env.n", a.n());
                emit("A.env.vx", a.velocity_x());
            }
        }
    }

    // ------------------------------------------------------------------
    // B. Two-volume transfer with bulk momentum and velocity integration
    // ------------------------------------------------------------------
    {
        let mut s0 = GasSystem::default();
        s0.initialize_default(
            units::pressure(15.0, units::PSI),
            units::volume(300.0, units::CC),
            units::celcius(25.0),
        );
        s0.set_geometry(
            units::distance(10.0, units::CM),
            units::distance(10.0, units::CM),
            1.0,
            0.0,
        );

        let mut s1 = GasSystem::default();
        s1.initialize_default(
            units::pressure(2.0, units::PSI),
            units::volume(1.0, units::L),
            units::celcius(25.0),
        );
        s1.set_geometry(
            units::distance(10.0, units::CM),
            units::distance(2.0, units::CM),
            1.0,
            0.0,
        );

        let params = FlowParameters {
            k_flow: GasSystem::k_28in_h2o(500.0) * 1.0,
            cross_section_area_0: 50.0 * units::CM * units::CM,
            cross_section_area_1: 4.0 * units::CM * units::CM,
            direction_x: 1.0,
            direction_y: 0.0,
            dt: 1.0 / 10000.0,
        };

        for i in 1..=1000 {
            GasSystem::flow(&mut s0, &mut s1, &params);
            s0.update_velocity_default(params.dt);
            s1.update_velocity_default(params.dt);
            if i % 200 == 0 {
                emit("B.P0", s0.pressure());
                emit("B.P1", s1.pressure());
                emit("B.vx0", s0.velocity_x());
                emit("B.vx1", s1.velocity_x());
                emit("B.E", s0.total_energy() + s1.total_energy());
                emit("B.n", s0.n() + s1.n());
            }
        }

        for _ in 0..500 {
            s0.dissipate_velocity(params.dt, 0.01);
            s1.dissipate_velocity(params.dt, 0.01);
        }

        emit("B.final.P0", s0.pressure());
        emit("B.final.P1", s1.pressure());
        emit("B.final.vx0", s0.velocity_x());
        emit("B.final.vx1", s1.velocity_x());
        emit("B.final.E", s0.total_energy() + s1.total_energy());
        emit("B.final.n", s0.n() + s1.n());
    }

    // ------------------------------------------------------------------
    // C. Degrees of freedom: dynamic pressure and excess-velocity clamping
    // ------------------------------------------------------------------
    {
        let dofs: [i32; 3] = [3, 5, 7];
        for dof in dofs {
            let mut src = GasSystem::default();
            src.initialize(
                units::pressure(2.0, units::ATM),
                units::volume(200.0, units::CC),
                units::celcius(25.0),
                Mix::new(0.0, 0.79, 0.21),
                dof,
            );
            src.set_geometry(
                units::distance(10.0, units::CM),
                units::distance(1.0, units::CM),
                1.0,
                0.0,
            );

            let mut dst = GasSystem::default();
            dst.initialize(
                units::pressure(1.0, units::ATM),
                units::volume(100.0, units::CC),
                units::celcius(25.0),
                Mix::new(0.0, 0.79, 0.21),
                dof,
            );
            dst.set_geometry(
                units::distance(10.0, units::CM),
                units::distance(1.0, units::CM),
                1.0,
                0.0,
            );

            let params = FlowParameters {
                k_flow: GasSystem::k_carb(300.0),
                cross_section_area_0: 3.0 * units::CM * units::CM,
                cross_section_area_1: 3.0 * units::CM * units::CM,
                direction_x: 1.0,
                direction_y: 0.0,
                dt: 1.0 / 20000.0,
            };

            for _ in 0..500 {
                GasSystem::flow(&mut src, &mut dst, &params);
            }

            emit("C.P", dst.pressure());
            emit("C.dp.x", dst.dynamic_pressure(1.0, 0.0));
            emit("C.dp.y", dst.dynamic_pressure(0.0, 1.0));
            emit("C.dp.-x", dst.dynamic_pressure(-1.0, 0.0));
            emit(
                "C.dp.diag",
                dst.dynamic_pressure(0.7071067811865476, -0.7071067811865476),
            );
            emit("C.vx", dst.velocity_x());
            emit("C.vy", dst.velocity_y());
            emit("C.c", dst.c());
            emit("C.E", dst.total_energy());
            emit("C.bulkEk", dst.bulk_kinetic_energy());

            dst.dissipate_excess_velocity();

            emit("C.diss.P", dst.pressure());
            emit("C.diss.vx", dst.velocity_x());
            emit("C.diss.c", dst.c());
            emit("C.diss.E", dst.total_energy());
            emit("C.diss.bulkEk", dst.bulk_kinetic_energy());
        }
    }

    // ------------------------------------------------------------------
    // D. Pairwise equilibrium flow
    // ------------------------------------------------------------------
    {
        let mut g1 = GasSystem::default();
        g1.initialize_default(
            units::pressure(1.0, units::ATM),
            units::volume(1.0, units::CC),
            units::celcius(2500.0),
        );
        let mut g2 = GasSystem::default();
        g2.initialize_default(
            units::pressure(2.0, units::ATM),
            units::volume(1.0, units::CC),
            units::celcius(25.0),
        );

        emit("D.maxFlow.a", g1.pressure_equilibrium_max_flow(&g2));
        emit("D.maxFlow.b", g2.pressure_equilibrium_max_flow(&g1));

        let mf = g1.pressure_equilibrium_max_flow(&g2);
        let e_k_per_mol = g1.kinetic_energy_per_mol();
        g1.lose_n(mf, e_k_per_mol);
        let mix = g1.mix();
        g2.gain_n(mf, e_k_per_mol, mix);

        emit("D.P1", g1.pressure());
        emit("D.P2", g2.pressure());
        emit("D.n1", g1.n());
        emit("D.n2", g2.n());
    }
}
