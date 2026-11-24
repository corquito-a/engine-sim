// Shared trace harness for the C++ reference drivers.
//
// Both `core_reference.cpp` (engine built by hand, matching the Rust fixture)
// and `script_reference.cpp` (engine built by running the `.mr` script through
// the original piranha + es_script stack) emit exactly the same fields from the
// same code, so the two drivers can be compared against each other as well as
// against the Rust side.
//
// Output format: `<step>,<field>,<16 hex digits of the f64 bit pattern>`.

#ifndef ENGINE_SIM_CORE_TRACE_COMMON_H
#define ENGINE_SIM_CORE_TRACE_COMMON_H

#include "combustion_chamber.h"
#include "crankshaft.h"
#include "cylinder_head.h"
#include "engine.h"
#include "exhaust_system.h"
#include "gas_system.h"
#include "ignition_module.h"
#include "intake.h"
#include "piston_engine_simulator.h"
#include "units.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>

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
/// integer arithmetic only so every implementation builds the identical table.
static void build_impulse_response(int16_t *ir, int samples) {
    for (int i = 0; i < samples; ++i) {
        const int value = ((i * 7919) % 4001) - 2000;
        const int envelope = 255 - (i % 256);
        ir[i] = (int16_t)(value * envelope / 256);
    }
}

/// Installs the deterministic impulse response on every exhaust channel.
static void install_impulse_response(PhysicsSimulator *simulator, Engine *engine) {
    int16_t ir[512];
    build_impulse_response(ir, 512);
    for (int i = 0; i < engine->getExhaustSystemCount(); ++i) {
        simulator->synthesizer().initializeImpulseResponse(ir, 512, 1.0f, i);
    }
}

/// Runs `steps` application frames, emitting the physics, the synthesizer input
/// and every rendered audio sample.
static void traceFrames(PhysicsSimulator *simulator, Engine *engine, int stepCount) {
    Crankshaft *crank = engine->getCrankshaft(0);
    Piston *piston = engine->getPiston(0);
    ConnectingRod *rod = engine->getConnectingRod(0);
    CylinderHead *head = engine->getHead(0);
    Intake *intake = engine->getIntake(0);
    ExhaustSystem *exhaust = engine->getExhaustSystem(0);
    CombustionChamber *chamber = engine->getChamber(0);

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
            const double attenuation =
                std::min(std::abs(simulator->filteredEngineSpeed()), 40.0) / 40.0;
            emit(step, "filtered_speed", simulator->filteredEngineSpeed());
            emit(step, "attenuation_3", attenuation * attenuation * attenuation);
            emit(step, "exhaust_dp",
                 chamber->m_exhaustRunnerAndPrimary.pressure() - units::pressure(1.0, units::atm));
            emit(step, "exhaust_dyn_p",
                 chamber->m_exhaustRunnerAndPrimary.dynamicPressure(1.0, 0.0));
            emit(step, "exhaust_dyn_n",
                 chamber->m_exhaustRunnerAndPrimary.dynamicPressure(-1.0, 0.0));
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

    (void)head;
}

#endif /* ENGINE_SIM_CORE_TRACE_COMMON_H */
