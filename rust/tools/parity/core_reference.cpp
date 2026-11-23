// Reference implementation half of the whole-core parity check.
//
// Builds the same single-cylinder engine as
// `crates/engine-sim-core/tests/fixture.rs` (a port of
// `assets/engines/atg-video-1/01_honda_trx520.mr`) using the *original* C++
// core, runs it, and prints one line per simulation step:
//
//     <step>,<field>,<16 hex digits of the f64 bit pattern>
//
// `examples/core_trace.rs` prints the same lines from the Rust port; the two
// outputs are compared bit-for-bit by `run_core_parity.sh`.
//
// The audio path is traced alongside the physics (see `renderBlock`).
// (stale note removed)
//// milestone): only the physics is traced.

#include "engine.h"
#include "transmission.h"
#include "vehicle.h"
#include "piston_engine_simulator.h"

#include "camshaft.h"
#include "combustion_chamber.h"
#include "connecting_rod.h"
#include "crankshaft.h"
#include "cylinder_bank.h"
#include "cylinder_head.h"
#include "direct_throttle_linkage.h"
#include "exhaust_system.h"
#include "fuel.h"
#include "function.h"
#include "gas_system.h"
#include "ignition_module.h"
#include "intake.h"
#include "piston.h"
#include "standard_valvetrain.h"
#include "units.h"

#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>

// ------------------------------------------------------------------ helpers

static void emit(int step, const char *name, double value) {
    unsigned long long bits;
    std::memcpy(&bits, &value, sizeof(bits));
    std::printf("%d,%s,%016llx\n", step, name, bits);
}

// `PistonEngineSimulator` with the real `writeToSynthesizer`, so the harness
// exercises the exhaust-flow signal and the synthesizer exactly as the
// application does.
class PhysicsSimulator : public PistonEngineSimulator {
public:
    /// `m_exhaustFlowStagingBuffer` is protected; the harness only reads it.
    double exhaustSignal(int i) const { return m_exhaustFlowStagingBuffer[i]; }
};

/// Body of `Synthesizer::renderAudio()` with the condition-variable wait
/// removed: the reference runs this on the audio thread, the harness drives it
/// from the frame loop so the output is reproducible.
static int renderBlock(Synthesizer &s) {
    const int n = std::min(
        std::max(0, 2000 - (int)s.m_audioBuffer.size()),
        (int)s.m_inputChannels[0].data.size());

    for (int i = 0; i < s.m_inputChannelCount; ++i) {
        s.m_inputChannels[i].data.read(
            (size_t)n, s.m_inputChannels[i].transferBuffer);
    }

    s.m_inputSamplesRead = n;
    s.m_processed = true;

    for (int i = 0; i < s.m_inputChannelCount; ++i) {
        s.m_filters[i].airNoiseLowPass.setCutoffFrequency(
            (float)s.m_audioParameters.airNoiseFrequencyCutoff, s.m_audioSampleRate);
        s.m_filters[i].jitterFilter.setJitterScale(
            s.m_audioParameters.inputSampleNoise);
    }

    for (int i = 0; i < n; ++i) {
        s.m_audioBuffer.write(s.renderAudio(i));
    }

    return n;
}

/// Deterministic impulse response for the exhaust convolution, generated with
/// integer arithmetic only so both implementations build the identical table.
static void build_impulse_response(int16_t *ir, int samples) {
    for (int i = 0; i < samples; ++i) {
        const int value = ((i * 7919) % 4001) - 2000;
        const int envelope = 255 - (i % 256);
        ir[i] = (int16_t)(value * envelope / 256);
    }
}

/// `Function` has no public `setFilterRadius` in the original (the scripting
/// layer's `FunctionNode` sets the protected member directly), so the harness
/// subclasses it the same way.
class ScriptFunction : public Function {
public:
    void setRadius(double radius) { m_filterRadius = radius; }
};

// ------------------------------------------------------- engine definition

// `harmonic_cam_lobe` (`GenerateHarmonicCamLobeNode::_evaluate`)
static void generate_harmonic_cam_lobe(
    Function *function,
    double durationAt50Thou,
    double gamma,
    double lift,
    int steps)
{
    const double angle = durationAt50Thou / 4;
    const double s =
        std::pow(2 * units::distance(50, units::thou) / lift, 1 / gamma) - 1;
    const double k = std::acos(s) / angle;
    const double extents = constants::pi / k;

    const double step = extents / (steps - 5.0);
    for (int i = 0; i < steps; ++i) {
        if (i == 0) {
            function->addSample(0.0, lift);
        }
        else {
            const double x = i * step;
            const double y = (x >= extents)
                ? 0.0
                : lift * std::pow(0.5 + 0.5 * std::cos(k * x), gamma);
            function->addSample(x, y);
            function->addSample(-x, y);
        }
    }

    static_cast<ScriptFunction *>(function)->setRadius(step);
}

struct HeadFlowSample {
    double liftThou;
    double flow;
};

static void build_port_flow(
    Function *function,
    const HeadFlowSample *samples,
    int count,
    double attenuation)
{
    for (int i = 0; i < count; ++i) {
        function->addSample(
            samples[i].liftThou * units::thou,
            GasSystem::k_28inH2O(samples[i].flow * attenuation));
    }
}

int main(int argc, char **argv) {
    const int stepCount = (argc > 1) ? std::atoi(argv[1]) : 2000;
    const double deg = units::deg;

    // ----------------------------------------------------------- engine shell
    Engine *engine = new Engine;
    engine->initialize(Engine::Parameters{
        /* cylinderBanks */ 1,
        /* cylinderCount */ 1,
        /* crankshaftCount */ 1,
        /* exhaustSystemCount */ 1,
        /* intakeCount */ 1,
        /* name */ std::string("Honda TRX520 (ATV)"),
        /* starterTorque */ units::torque(50.0, units::ft_lb),
        /* starterSpeed */ units::rpm(500.0),
        /* redline */ units::rpm(5000.0),
        /* dynoMinSpeed */ units::rpm(1000.0),
        /* dynoMaxSpeed */ units::rpm(6500.0),
        /* dynoHoldStep */ units::rpm(100),
        /* throttle */ new DirectThrottleLinkage,
        /* initialSimulationFrequency */ 10000.0,
        /* initialHighFrequencyGain */ 0.01,
        /* initialNoise */ 1.0,
        /* initialJitter */ 0.5,
    });

    engine->setSpeedControl(1.0); // wide open throttle

    Crankshaft *crank = engine->getCrankshaft(0);
    Crankshaft::Parameters crankParams;
    crankParams.mass = units::mass(5.0, units::lb);
    crankParams.flywheelMass = units::mass(5.0, units::lb);
    crankParams.momentOfInertia = 0.22986844776863666 * 0.2;
    crankParams.crankThrow = units::distance(71.5, units::mm) / 2.0;
    crankParams.rodJournals = 1;
    crankParams.pos_x = 0.0;
    crankParams.pos_y = 0.0;
    crankParams.tdc = constants::pi / 2;
    crankParams.frictionTorque = units::torque(5.0, units::ft_lb);
    crank->initialize(crankParams);
    crank->setRodJournalAngle(0, 0.0);

    CylinderBank *bank = engine->getCylinderBank(0);
    CylinderBank::Parameters bankParams;
    bankParams.angle = 0.0;
    bankParams.bore = units::distance(96.0, units::mm);
    bankParams.deckHeight =
        units::distance(71.5, units::mm) / 2
        + units::distance(6.0, units::inch)
        + units::distance(1.0, units::inch);
    bankParams.cylinderCount = 1;
    bankParams.positionX = 0.0;
    bankParams.positionY = 0.0;
    bankParams.displayDepth = 0.4;
    bankParams.index = 0;
    bank->initialize(bankParams);

    // ---------------------------------------------------------------- intake
    Intake *intake = engine->getIntake(0);
    Intake::Parameters intakeParams;
    intakeParams.volume = units::volume(1.5, units::L);
    intakeParams.CrossSectionArea = units::area(10.0, units::cm2);
    intakeParams.InputFlowK = GasSystem::k_carb(100.0);
    intakeParams.IdleFlowK = GasSystem::k_carb(0.0);
    intakeParams.RunnerFlowRate = GasSystem::k_carb(300.0);
    intakeParams.IdleThrottlePlatePosition = 0.993;
    intakeParams.VelocityDecay = 0.5;
    intake->initialize(intakeParams);

    // -------------------------------------------------------- exhaust system
    ExhaustSystem *exhaust = engine->getExhaustSystem(0);
    const double exhaustArea =
        constants::pi * std::pow(units::distance(2.0, units::inch), 2.0);
    ExhaustSystem::Parameters exhaustParams;
    exhaustParams.length = units::volume(5.0, units::L) / exhaustArea;
    exhaustParams.collectorCrossSectionArea = exhaustArea;
    exhaustParams.outletFlowRate = GasSystem::k_carb(500.0);
    exhaustParams.primaryTubeLength = units::distance(20.0, units::inch);
    exhaustParams.primaryFlowRate = GasSystem::k_carb(200.0);
    exhaustParams.velocityDecay = 0.5;
    exhaustParams.audioVolume = 1.0;
    exhaustParams.impulseResponse = nullptr;
    exhaust->initialize(exhaustParams);

    // ------------------------------------------------------------ valve train
    ScriptFunction *lobe = new ScriptFunction;
    lobe->initialize(201, 0.0);
    generate_harmonic_cam_lobe(
        lobe,
        units::angle(180.0, deg),
        1.0,
        units::distance(200.0, units::thou),
        100);

    // Allocated on the heap: the destructors assert that `destroy()` was
    // called, and the simulator does not own these objects.
    Camshaft *intakeCam = new Camshaft;
    Camshaft *exhaustCam = new Camshaft;
    const double lobeSeparation = units::angle(100.0, deg);
    const double rot360 = units::angle(360.0, deg);

    Camshaft::Parameters camParams;
    camParams.lobes = 1;
    camParams.crankshaft = crank;
    camParams.lobeProfile = lobe;
    camParams.advance = 0.0;
    camParams.baseRadius = units::distance(500.0, units::thou);

    intakeCam->initialize(camParams);
    intakeCam->setLobeCenterline(0, rot360 + lobeSeparation);

    exhaustCam->initialize(camParams);
    exhaustCam->setLobeCenterline(0, rot360 - lobeSeparation);

    StandardValvetrain *valvetrain = new StandardValvetrain;
    StandardValvetrain::Parameters valvetrainParams;
    valvetrainParams.intakeCamshaft = intakeCam;
    valvetrainParams.exhaustCamshaft = exhaustCam;
    valvetrain->initialize(valvetrainParams);

    // Intake/exhaust port flow tables (`generic_small_engine_head`,
    // `flow_attenuation: 2.0`).
    static const HeadFlowSample intakeSamples[] = {
        { 0.0, 0.0 },   { 50.0, 25.0 },  { 100.0, 75.0 },  { 150.0, 100.0 },
        { 200.0, 130.0 }, { 250.0, 180.0 }, { 300.0, 190.0 }, { 350.0, 220.0 },
        { 400.0, 240.0 }, { 450.0, 250.0 }, { 500.0, 260.0 }, { 550.0, 260.0 },
        { 600.0, 260.0 }, { 650.0, 255.0 }, { 700.0, 250.0 },
    };

    static const HeadFlowSample exhaustSamples[] = {
        { 0.0, 0.0 },   { 50.0, 25.0 },  { 100.0, 50.0 },  { 150.0, 75.0 },
        { 200.0, 100.0 }, { 250.0, 125.0 }, { 300.0, 160.0 }, { 350.0, 175.0 },
        { 400.0, 180.0 }, { 450.0, 190.0 }, { 500.0, 200.0 }, { 550.0, 205.0 },
        { 600.0, 210.0 }, { 650.0, 210.0 }, { 700.0, 210.0 },
    };

    Function *intakeFlow = new Function;
    intakeFlow->initialize(15, units::distance(50.0, units::thou));
    build_port_flow(
        intakeFlow,
        intakeSamples,
        sizeof(intakeSamples) / sizeof(intakeSamples[0]),
        2.0);

    Function *exhaustFlow = new Function;
    exhaustFlow->initialize(15, units::distance(50.0, units::thou));
    build_port_flow(
        exhaustFlow,
        exhaustSamples,
        sizeof(exhaustSamples) / sizeof(exhaustSamples[0]),
        2.0);

    CylinderHead *head = engine->getHead(0);
    CylinderHead::Parameters headParams;
    headParams.Bank = bank;
    headParams.Valvetrain = valvetrain;
    headParams.IntakePortFlow = intakeFlow;
    headParams.ExhaustPortFlow = exhaustFlow;
    headParams.CombustionChamberVolume = units::volume(60.0, units::cc);
    headParams.IntakeRunnerVolume = units::volume(100.0, units::cc);
    headParams.IntakeRunnerCrossSectionArea = units::area(30.0, units::cm2);
    headParams.ExhaustRunnerVolume = units::volume(100.0, units::cc);
    headParams.ExhaustRunnerCrossSectionArea = units::area(30.0, units::cm2);
    headParams.FlipDisplay = false;
    head->initialize(headParams);
    head->setIntake(0, intake);
    head->setExhaustSystem(0, exhaust);

    // --------------------------------------------------------- rod and piston
    ConnectingRod *rod = engine->getConnectingRod(0);
    Piston *piston = engine->getPiston(0);

    ConnectingRod::Parameters rodParams;
    rodParams.centerOfMass = 0.0;
    rodParams.length = units::distance(6.0, units::inch);
    rodParams.mass = units::mass(100.0, units::g);
    rodParams.momentOfInertia = 0.0015884918028487504;
    rodParams.journal = 0;
    rodParams.crankshaft = crank;
    rodParams.piston = piston;
    rodParams.slaveThrow = 0.0;
    rodParams.rodJournals = 1;
    rodParams.master = nullptr;
    rod->initialize(rodParams);

    Piston::Parameters pistonParams;
    pistonParams.Rod = rod;
    pistonParams.Bank = bank;
    pistonParams.CylinderIndex = 0;
    pistonParams.CompressionHeight = units::distance(1.0, units::inch);
    pistonParams.Displacement = 0.0;
    pistonParams.WristPinPosition = 0.0;
    pistonParams.mass = units::mass(100.0, units::g);
    pistonParams.BlowbyFlowCoefficient = GasSystem::k_28inH2O(0.1);
    piston->initialize(pistonParams);

    // ---------------------------------------------------------------- fuel
    Function *turbulenceFlameSpeed = new Function;
    turbulenceFlameSpeed->initialize(10, 5.0);
    for (int i = 0; i < 10; ++i) {
        const double s = 5.0 * i;
        turbulenceFlameSpeed->addSample(s, 1.5 * s);
    }
    turbulenceFlameSpeed->addSample(0.0, 3.0);

    Fuel::Parameters fuelParams;
    fuelParams.molecularMass = units::mass(100.0, units::g);
    fuelParams.energyDensity = units::energy(48.1, units::kJ) / units::mass(1.0, units::g);
    fuelParams.density = units::mass(0.755, units::kg) / units::volume(1.0, units::L);
    fuelParams.turbulenceToFlameSpeedRatio = turbulenceFlameSpeed;
    fuelParams.molecularAfr = 25.0 / 2.0;
    fuelParams.burningEfficiencyRandomness = 0.5;
    fuelParams.maxBurningEfficiency = 0.8;
    fuelParams.maxDilutionEffect = 10.0;
    fuelParams.maxTurbulenceEffect = 2.0;
    fuelParams.lowEfficiencyAttenuation = 0.6;
    engine->getFuel()->initialize(fuelParams);

    // -------------------------------------------------------------- ignition
    Function *timingCurve = new Function;
    timingCurve->initialize(5, units::rpm(1000.0));
    timingCurve->addSample(units::rpm(0.0), units::angle(12.0, deg));
    timingCurve->addSample(units::rpm(1000.0), units::angle(12.0, deg));
    timingCurve->addSample(units::rpm(2000.0), units::angle(20.0, deg));
    timingCurve->addSample(units::rpm(3000.0), units::angle(35.0, deg));
    timingCurve->addSample(units::rpm(4000.0), units::angle(35.0, deg));

    IgnitionModule::Parameters ignitionParams;
    ignitionParams.cylinderCount = 1;
    ignitionParams.crankshaft = crank;
    ignitionParams.timingCurve = timingCurve;
    ignitionParams.revLimit = units::rpm(6000.0);
    ignitionParams.limiterDuration = 0.5 * units::sec;
    engine->getIgnitionModule()->initialize(ignitionParams);
    engine->getIgnitionModule()->setFiringOrder(0, units::angle(0.0, deg));
    engine->getIgnitionModule()->m_enabled = true;

    // -------------------------------------------------------------- chambers
    Function *meanPistonSpeedToTurbulence = new Function;
    meanPistonSpeedToTurbulence->initialize(30, 1);
    for (int i = 0; i < 30; ++i) {
        const double s = (double)i;
        meanPistonSpeedToTurbulence->addSample(s, s * 0.5);
    }

    CombustionChamber::Parameters ccParams;
    ccParams.Piston = piston;
    ccParams.Head = head;
    ccParams.Fuel = engine->getFuel();
    ccParams.MeanPistonSpeedToTurbulence = meanPistonSpeedToTurbulence;
    ccParams.StartingPressure = units::pressure(1.0, units::atm);
    ccParams.StartingTemperature = units::celcius(25.0);
    ccParams.CrankcasePressure = units::pressure(1.0, units::atm);
    engine->getChamber(0)->initialize(ccParams);

    // ------------------------------------------------------- vehicle + trans
    Vehicle *vehicle = new Vehicle;
    Vehicle::Parameters vehicleParams;
    vehicleParams.mass = units::mass(500.0, units::kg);
    vehicleParams.dragCoefficient = 0.25;
    vehicleParams.crossSectionArea =
        units::distance(47.0, units::inch) * units::distance(47.0, units::inch);
    vehicleParams.diffRatio = 3.33;
    vehicleParams.tireRadius = units::distance(11.0, units::inch);
    vehicleParams.rollingResistance = units::force(200.0, units::N);
    vehicle->initialize(vehicleParams);

    Transmission *transmission = new Transmission;
    const double gearRatios[] = { 4.0, 3.5, 3.0, 2.5, 2.0 };
    Transmission::Parameters transmissionParams;
    transmissionParams.GearCount = 5;
    transmissionParams.MaxClutchTorque = units::torque(50.0, units::ft_lb);
    transmissionParams.GearRatios = gearRatios;
    transmission->initialize(transmissionParams);

    // --------------------------------------------------------------- simulate
    PhysicsSimulator *simulator = new PhysicsSimulator;
    Simulator::Parameters simulatorParams;
    simulatorParams.systemType = Simulator::SystemType::NsvOptimized;
    simulator->initialize(simulatorParams);
    simulator->loadSimulation(engine, vehicle, transmission);
    simulator->setFluidSimulationSteps(8);
    simulator->setSimulationFrequency(10000);
    simulator->m_starterMotor.m_enabled = true;

    // Parameter switches used to bisect DSP mismatches.
    {
        Synthesizer::AudioParameters ap =
            simulator->synthesizer().getAudioParameters();
        if (std::getenv("PARITY_NO_JITTER")) ap.inputSampleNoise = 0.0f;
        if (std::getenv("PARITY_NO_AIR_NOISE")) ap.airNoise = 0.0f;
        if (std::getenv("PARITY_NO_CONV")) ap.convolution = 0.0f;
        simulator->synthesizer().setAudioParameters(ap);
    }

    // Deterministic exhaust impulse response for the convolution stage.
    {
        int16_t ir[512];
        build_impulse_response(ir, 512);
        for (int i = 0; i < engine->getExhaustSystemCount(); ++i) {
            simulator->synthesizer().initializeImpulseResponse(ir, 512, 1.0f, i);
        }
    }

    // Optional dynamometer coverage: `PARITY_DYNO=1` holds the crank at a
    // fixed speed instead of free-revving it.
    if (std::getenv("PARITY_DYNO") != nullptr) {
        simulator->m_dyno.m_enabled = true;
        simulator->m_dyno.m_hold = true;
        simulator->m_dyno.m_rotationSpeed = units::rpm(-1200.0);
    }

    const double timestep = simulator->getTimestep();

    for (int step = 0; step < stepCount; ++step) {
        // `startFrame` with a sub-half-timestep `dt` yields exactly one
        // simulation step per frame (the latency heuristic then bumps a
        // zero-step frame to one).
        simulator->startFrame(timestep * 0.1);
        while (simulator->simulateStep()) { /* run the frame */ }
        simulator->endFrame();

        {
            Synthesizer &syn = simulator->synthesizer();
            const int n = std::min(
                std::max(0, 2000 - (int)syn.m_audioBuffer.size()),
                (int)syn.m_inputChannels[0].data.size());
            for (int i = 0; i < syn.m_inputChannelCount; ++i) {
                syn.m_inputChannels[i].data.read(
                    (size_t)n, syn.m_inputChannels[i].transferBuffer);
            }
            char name[32];
            for (int i = 0; i < n; ++i) {
                std::snprintf(name, sizeof(name), "in%d", i);
                emit(step, name, (double)syn.m_inputChannels[0].transferBuffer[i]);
            }
        }

        renderBlock(simulator->synthesizer());

        {
            int16_t audio[4096];
            const int produced = simulator->readAudioOutput(4096, audio);
            char name[32];
            for (int i = 0; i < produced; ++i) {
                std::snprintf(name, sizeof(name), "audio%d", i);
                emit(step, name, (double)audio[i]);
            }
        }

        atg_scs::RigidBody &crankBody = crank->m_body;
        atg_scs::RigidBody &pistonBody = piston->m_body;
        atg_scs::RigidBody &rodBody = rod->m_body;
        CombustionChamber *chamber = engine->getChamber(0);

        emit(step, "crank_theta", crankBody.theta);
        emit(step, "crank_v_theta", crankBody.v_theta);
        emit(step, "piston_p_x", pistonBody.p_x);
        emit(step, "piston_p_y", pistonBody.p_y);
        emit(step, "piston_v_x", pistonBody.v_x);
        emit(step, "piston_v_y", pistonBody.v_y);
        emit(step, "rod_theta", rodBody.theta);
        emit(step, "rod_p_x", rodBody.p_x);
        emit(step, "rod_p_y", rodBody.p_y);
        emit(step, "chamber_pressure", chamber->m_system.pressure());
        emit(step, "chamber_temperature", chamber->m_system.temperature());
        emit(step, "chamber_n", chamber->m_system.n());
        emit(step, "chamber_mix_fuel", chamber->m_system.mix().p_fuel);
        emit(step, "chamber_mix_o2", chamber->m_system.mix().p_o2);
        emit(step, "intake_pressure", intake->m_system.pressure());
        emit(step, "runner_pressure", chamber->m_intakeRunnerAndManifold.pressure());
        emit(step, "exhaust_pressure", exhaust->getSystem()->pressure());
        emit(step, "crank_cycle_angle", crank->getCycleAngle());
        emit(step, "ignition_advance",
             engine->getIgnitionModule()->getTimingAdvance());
        emit(step, "chamber_mix_inert", chamber->m_system.mix().p_inert);
        emit(step, "chamber_volume", chamber->getVolume());
        emit(step, "chamber_lit", chamber->isLit() ? 1.0 : 0.0);
        emit(step, "chamber_afr", chamber->lastEventAfr());
        emit(step, "chamber_exhaust_flow", chamber->getLastIterationExhaustFlow());
        {
            CombustionChamber *ch = engine->getChamber(0);
            const double attenuation =
                std::min(std::abs(simulator->filteredEngineSpeed()), 40.0) / 40.0;
            emit(step, "filtered_speed", simulator->filteredEngineSpeed());
            emit(step, "attenuation_3", attenuation * attenuation * attenuation);
            emit(step, "exhaust_dp",
                 ch->m_exhaustRunnerAndPrimary.pressure() - units::pressure(1.0, units::atm));
            emit(step, "exhaust_dyn_p",
                 ch->m_exhaustRunnerAndPrimary.dynamicPressure(1.0, 0.0));
            emit(step, "exhaust_dyn_n",
                 ch->m_exhaustRunnerAndPrimary.dynamicPressure(-1.0, 0.0));
        }
        emit(step, "exhaust_signal", simulator->exhaustSignal(0));
        emit(step, "synth_latency", simulator->getSynthesizerInputLatency());
        emit(step, "synth_channels",
             (double)simulator->synthesizer().m_inputChannelCount);
        emit(step, "synth_input_size",
             (double)simulator->synthesizer().m_inputChannels[0].data.size());
        emit(step, "synth_write_offset",
             simulator->synthesizer().m_inputWriteOffset);
        emit(step, "frame_steps", (double)simulator->simulationSteps());
        emit(step, "dyno_torque", simulator->m_dyno.getTorque());
    }

    return 0;
}
