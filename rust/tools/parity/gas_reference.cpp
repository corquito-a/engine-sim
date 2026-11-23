// Reference trace generator for the gas-system port.
//
// Compiles the *original* C++ sources (`src/gas_system.cpp`,
// `src/utilities.cpp`) and runs the same scripted scenario as
// `crates/engine-sim-core/examples/gas_trace.rs`, printing every checkpoint as
// the raw IEEE-754 bit pattern of the `double`. The two outputs are diffed by
// `run_parity.sh`: matching bit patterns mean the port is numerically
// identical to the original, not merely "close".
//
// Build: see run_parity.sh

#include "gas_system.h"
#include "units.h"

#include <cstdio>
#include <cstring>

// The C++ header relies on MSVC accepting `const Mix &mix = {}` default
// arguments inside a class definition; GCC/Clang reject that (see
// run_parity.sh, which strips those defaults). Every call site therefore passes
// the mixture explicitly -- `kDefault` is exactly `Mix{}`.
static const GasSystem::Mix kDefault{0.0, 1.0, 0.0};

static void emit(const char *name, double v) {
    unsigned long long bits;
    std::memcpy(&bits, &v, sizeof(bits));
    std::printf("%s,%016llx\n", name, bits);
}

int main() {
    // ------------------------------------------------------------------
    // A. Single-volume thermodynamics
    // ------------------------------------------------------------------
    {
        GasSystem a;
        a.initialize(
            units::pressure(1.0, units::atm),
            units::volume(1.0, units::cc),
            units::celcius(25.0),
            kDefault);

        emit("A.n", a.n());
        emit("A.P", a.pressure());
        emit("A.T", a.temperature());
        emit("A.Ek", a.kineticEnergy());
        emit("A.EkPerMol", a.kineticEnergyPerMol());
        emit("A.mass", a.mass());
        emit("A.rho", a.approximateDensity());
        emit("A.c", a.c());
        emit("A.hcr", a.heatCapacityRatio());
        emit("A.totalE", a.totalEnergy());
        emit("A.bulkEk", a.bulkKineticEnergy());
        emit("A.vx", a.velocity_x());
        emit("A.vy", a.velocity_y());
        emit("A.dp.1.0", a.dynamicPressure(1.0, 0.0));
        emit("A.chokedLimit", GasSystem::chokedFlowLimit(5));
        emit("A.chokedRate", GasSystem::chokedFlowRate(5));
        emit("A.k28", GasSystem::k_28inH2O(300.0));
        emit("A.kcarb", GasSystem::k_carb(500.0));
        emit("A.flowConstant", GasSystem::flowConstant(
            units::flow(400, units::scfm),
            units::pressure(2.5, units::atm),
            units::pressure(1.5, units::atm),
            units::celcius(2000.0),
            GasSystem::heatCapacityRatio(5)));
        emit("A.flowRate.choked", GasSystem::flowRate(
            GasSystem::k_carb(400.0),
            units::pressure(2.5, units::atm),
            units::pressure(1.0, units::atm),
            units::celcius(2000.0),
            units::celcius(25.0),
            GasSystem::heatCapacityRatio(5),
            GasSystem::chokedFlowLimit(5),
            GasSystem::chokedFlowRate(5)));
        emit("A.flowRate.subcritical", GasSystem::flowRate(
            GasSystem::k_carb(400.0),
            units::pressure(2.5, units::atm),
            units::pressure(2.0, units::atm),
            units::celcius(2000.0),
            units::celcius(25.0),
            GasSystem::heatCapacityRatio(5),
            GasSystem::chokedFlowLimit(5),
            GasSystem::chokedFlowRate(5)));
        emit("A.flowRate.zeroK", GasSystem::flowRate(
            0.0,
            units::pressure(2.5, units::atm),
            units::pressure(2.0, units::atm),
            units::celcius(2000.0),
            units::celcius(25.0),
            GasSystem::heatCapacityRatio(5),
            GasSystem::chokedFlowLimit(5),
            GasSystem::chokedFlowRate(5)));

        // Compression stroke (100 steps, emit every 10th).
        const double dV = units::volume(0.5, units::cc);
        for (int i = 0; i < 100; ++i) {
            a.changeVolume(-dV / 100.0);
            if (i % 10 == 9) {
                emit("A.comp.P", a.pressure());
                emit("A.comp.T", a.temperature());
                emit("A.comp.Ek", a.kineticEnergy());
            }
        }

        a.changePressure(units::pressure(0.5, units::atm));
        emit("A.chgP.P", a.pressure());
        a.changeTemperature(50.0);
        emit("A.chgT.T", a.temperature());
        a.changeTemperature(10.0, a.n() * 0.1);
        emit("A.chgTN.T", a.temperature());
        a.changeEnergy(0.25);
        emit("A.chgE.Ek", a.kineticEnergy());

        a.injectFuel(a.n() * 0.02);
        emit("A.inject.pFuel", a.mix().p_fuel);
        emit("A.inject.nFuel", a.n_fuel());

        const double reacted = a.react(a.n() * 0.5, GasSystem::Mix{0.0, 0.79, 0.21});
        emit("A.react.return", reacted);
        emit("A.react.n", a.n());
        emit("A.react.pFuel", a.mix().p_fuel);
        emit("A.react.pInert", a.mix().p_inert);
        emit("A.react.pO2", a.mix().p_o2);
        emit("A.react.P", a.pressure());
        emit("A.react.T", a.temperature());

        const double e_k_per_mol = a.kineticEnergyPerMol();
        a.gainN(a.n() * 0.3, e_k_per_mol, GasSystem::Mix{0.1, 0.8, 0.1});
        emit("A.gain.n", a.n());
        emit("A.gain.pFuel", a.mix().p_fuel);
        emit("A.gain.pInert", a.mix().p_inert);
        emit("A.gain.pO2", a.mix().p_o2);

        const double e_k_per_mol_2 = a.kineticEnergyPerMol();
        a.loseN(a.n() * 0.2, e_k_per_mol_2);
        emit("A.lose.n", a.n());
        emit("A.lose.T", a.temperature());

        emit("A.maxFlow.out", a.pressureEquilibriumMaxFlow(
            units::pressure(1.0, units::atm), units::celcius(25.0)));
        emit("A.maxFlow.in", a.pressureEquilibriumMaxFlow(
            units::pressure(6.0, units::atm), units::celcius(25.0)));

        for (int i = 0; i < 100; ++i) {
            const double f = a.flow(
                GasSystem::k_28inH2O(200.0),
                1.0 / 10000.0,
                units::pressure(1.0, units::atm),
                units::celcius(25.0),
                kDefault);
            if (i % 20 == 19) {
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
        GasSystem s0, s1;
        s0.initialize(
            units::pressure(15.0, units::psi),
            units::volume(300.0, units::cc),
            units::celcius(25.0),
            kDefault);
        s0.setGeometry(
            units::distance(10.0, units::cm),
            units::distance(10.0, units::cm),
            1.0,
            0.0);

        s1.initialize(
            units::pressure(2.0, units::psi),
            units::volume(1.0, units::L),
            units::celcius(25.0),
            kDefault);
        s1.setGeometry(
            units::distance(10.0, units::cm),
            units::distance(2.0, units::cm),
            1.0,
            0.0);

        GasSystem::FlowParameters params;
        params.k_flow = GasSystem::k_28inH2O(500.0) * 1.0;
        params.crossSectionArea_0 = 50.0 * units::cm * units::cm;
        params.crossSectionArea_1 = 4.0 * units::cm * units::cm;
        params.direction_x = 1.0;
        params.direction_y = 0.0;
        params.dt = 1 / 10000.0;
        params.system_0 = &s0;
        params.system_1 = &s1;

        for (int i = 1; i <= 1000; ++i) {
            GasSystem::flow(params);
            s0.updateVelocity(params.dt);
            s1.updateVelocity(params.dt);
            if (i % 200 == 0) {
                emit("B.P0", s0.pressure());
                emit("B.P1", s1.pressure());
                emit("B.vx0", s0.velocity_x());
                emit("B.vx1", s1.velocity_x());
                emit("B.E", s0.totalEnergy() + s1.totalEnergy());
                emit("B.n", s0.n() + s1.n());
            }
        }

        for (int i = 0; i < 500; ++i) {
            s0.dissipateVelocity(params.dt, 0.01);
            s1.dissipateVelocity(params.dt, 0.01);
        }

        emit("B.final.P0", s0.pressure());
        emit("B.final.P1", s1.pressure());
        emit("B.final.vx0", s0.velocity_x());
        emit("B.final.vx1", s1.velocity_x());
        emit("B.final.E", s0.totalEnergy() + s1.totalEnergy());
        emit("B.final.n", s0.n() + s1.n());
    }

    // ------------------------------------------------------------------
    // C. Degrees of freedom: dynamic pressure and excess-velocity clamping
    // ------------------------------------------------------------------
    {
        const int dofs[3] = {3, 5, 7};
        for (int k = 0; k < 3; ++k) {
            const int dof = dofs[k];

            GasSystem src, dst;
            src.initialize(
                units::pressure(2.0, units::atm),
                units::volume(200.0, units::cc),
                units::celcius(25.0),
                GasSystem::Mix{0.0, 0.79, 0.21},
                dof);
            src.setGeometry(
                units::distance(10.0, units::cm),
                units::distance(1.0, units::cm),
                1.0,
                0.0);

            dst.initialize(
                units::pressure(1.0, units::atm),
                units::volume(100.0, units::cc),
                units::celcius(25.0),
                GasSystem::Mix{0.0, 0.79, 0.21},
                dof);
            dst.setGeometry(
                units::distance(10.0, units::cm),
                units::distance(1.0, units::cm),
                1.0,
                0.0);

            GasSystem::FlowParameters params;
            params.k_flow = GasSystem::k_carb(300.0);
            params.crossSectionArea_0 = 3.0 * units::cm * units::cm;
            params.crossSectionArea_1 = 3.0 * units::cm * units::cm;
            params.direction_x = 1.0;
            params.direction_y = 0.0;
            params.dt = 1 / 20000.0;
            params.system_0 = &src;
            params.system_1 = &dst;

            for (int i = 0; i < 500; ++i) {
                GasSystem::flow(params);
            }

            emit("C.P", dst.pressure());
            emit("C.dp.x", dst.dynamicPressure(1.0, 0.0));
            emit("C.dp.y", dst.dynamicPressure(0.0, 1.0));
            emit("C.dp.-x", dst.dynamicPressure(-1.0, 0.0));
            emit("C.dp.diag", dst.dynamicPressure(0.7071067811865476, -0.7071067811865476));
            emit("C.vx", dst.velocity_x());
            emit("C.vy", dst.velocity_y());
            emit("C.c", dst.c());
            emit("C.E", dst.totalEnergy());
            emit("C.bulkEk", dst.bulkKineticEnergy());

            dst.dissipateExcessVelocity();

            emit("C.diss.P", dst.pressure());
            emit("C.diss.vx", dst.velocity_x());
            emit("C.diss.c", dst.c());
            emit("C.diss.E", dst.totalEnergy());
            emit("C.diss.bulkEk", dst.bulkKineticEnergy());
        }
    }

    // ------------------------------------------------------------------
    // D. Pairwise equilibrium flow
    // ------------------------------------------------------------------
    {
        GasSystem g1, g2;
        g1.initialize(
            units::pressure(1.0, units::atm),
            units::volume(1.0, units::cc),
            units::celcius(2500.0),
            kDefault);
        g2.initialize(
            units::pressure(2.0, units::atm),
            units::volume(1.0, units::cc),
            units::celcius(25.0),
            kDefault);

        emit("D.maxFlow.a", g1.pressureEquilibriumMaxFlow(&g2));
        emit("D.maxFlow.b", g2.pressureEquilibriumMaxFlow(&g1));

        const double mf = g1.pressureEquilibriumMaxFlow(&g2);
        const double e_k_per_mol = g1.kineticEnergyPerMol();
        g1.loseN(mf, e_k_per_mol);
        g2.gainN(mf, e_k_per_mol, g1.mix());

        emit("D.P1", g1.pressure());
        emit("D.P2", g2.pressure());
        emit("D.n1", g1.n());
        emit("D.n2", g2.n());
    }

    return 0;
}
