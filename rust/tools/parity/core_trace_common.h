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
#include <string>
#include <vector>

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

/// `ysWindowsAudioWaveFile`, without `mmio`: find `fmt `, find `data`, and read
/// the data segment as raw 16-bit PCM. `bitsPerSample` is derived from
/// `blockAlign / channels * 8` and the sample count is `dataSize / blockAlign`
/// -- which is *frames*, and is what `EngineSimApplication` passes to
/// `initializeImpulseResponse` as an `int16_t *`.
struct WaveFile {
    int channels = 0;
    int sampleRate = 0;
    int bitsPerSample = 0;
    int frames = 0;
    std::vector<int16_t> samples;

    bool open(const std::string &path) {
        std::FILE *file = std::fopen(path.c_str(), "rb");
        if (file == nullptr) return false;

        std::vector<char> bytes;
        char buffer[65536];
        size_t read = 0;
        while ((read = std::fread(buffer, 1, sizeof(buffer), file)) > 0) {
            bytes.insert(bytes.end(), buffer, buffer + read);
        }
        std::fclose(file);

        if (bytes.size() < 12 ||
            std::memcmp(&bytes[0], "RIFF", 4) != 0 ||
            std::memcmp(&bytes[8], "WAVE", 4) != 0) {
            return false;
        }

        bool haveFormat = false;
        int blockAlign = 0;
        size_t dataOffset = 0, dataSize = 0;

        size_t offset = 12;
        while (offset + 8 <= bytes.size()) {
            const char *id = &bytes[offset];
            uint32_t size = 0;
            std::memcpy(&size, &bytes[offset + 4], 4);
            const size_t body = offset + 8;
            const size_t end = std::min(body + size, bytes.size());

            if (std::memcmp(id, "fmt ", 4) == 0 && size >= 16) {
                uint16_t channels = 0, alignment = 0;
                uint32_t rate = 0;
                std::memcpy(&channels, &bytes[body + 2], 2);
                std::memcpy(&rate, &bytes[body + 4], 4);
                std::memcpy(&alignment, &bytes[body + 12], 2);
                this->channels = channels;
                this->sampleRate = (int)rate;
                blockAlign = alignment;
                haveFormat = true;
            }
            else if (std::memcmp(id, "data", 4) == 0) {
                dataOffset = body;
                dataSize = end - body;
            }

            // A chunk's size is padded to an even boundary.
            offset = body + size + (size & 1);
        }

        if (!haveFormat || blockAlign <= 0 || dataSize == 0) return false;

        this->bitsPerSample = (blockAlign / this->channels) * 8;
        this->frames = (int)(dataSize / (size_t)blockAlign);

        this->samples.resize(this->frames);
        for (int i = 0; i < this->frames; ++i) {
            std::memcpy(&this->samples[i], &bytes[dataOffset + i * 2], 2);
        }

        return true;
    }
};

/// FNV-1a over the decoded samples: how the wave gate compares two decoders
/// without printing tens of thousands of lines.
static uint64_t wave_hash(const WaveFile &wave) {
    uint64_t hash = 0xcbf29ce484222325ull;

    for (int16_t sample : wave.samples) {
        for (int i = 0; i < 2; ++i) {
            const uint8_t byte = (uint8_t)((uint16_t)sample >> (8 * i));
            hash ^= byte;
            hash *= 0x100000001b3ull;
        }
    }

    return hash;
}

/// Installs the impulse response each exhaust names, and the deterministic one
/// on any exhaust that names none. This is `EngineSimApplication::loadScript`'s
/// audio setup: open the file, hand its samples to the synthesizer with the
/// node's volume.
static void install_impulse_response(PhysicsSimulator *simulator, Engine *engine) {
    int16_t fallback[512];
    build_impulse_response(fallback, 512);

    for (int i = 0; i < engine->getExhaustSystemCount(); ++i) {
        ImpulseResponse *response = engine->getExhaustSystem(i)->getImpulseResponse();

        WaveFile wave;
        if (response != nullptr && wave.open(response->getFilename())) {
            simulator->synthesizer().initializeImpulseResponse(
                wave.samples.data(), (unsigned int)wave.frames,
                (float)response->getVolume(), i);
        }
        else {
            simulator->synthesizer().initializeImpulseResponse(fallback, 512, 1.0f, i);
        }
    }
}

/// How a run is driven. The defaults are the ones every gate uses: the starter
/// held (as the application holds `S`) and the script's own throttle, untouched.
///
/// `throttle` is a plate position, applied through the engine's speed control the
/// way the application applies its keys (`position = 1 - speed_control^gamma`), and
/// `starter_seconds` releases the starter after that much engine time -- without
/// it the starter's velocity constraint holds the crankshaft at the starter's
/// speed forever. `dyno_rpm` holds the crankshaft instead, and `ignition` (-1 =
/// leave alone) switches the ignition module, which is built disabled.
struct TraceOptions {
    double throttle = -1.0;
    double starter_seconds = -1.0;
    double dyno_rpm = -1.0;
    int ignition = -1;
    double clutch = -1.0;

    /// Gear, counted the way the command line counts: 1 is first, 0 is neutral.
    /// `-1` leaves the transmission in the neutral it starts in.
    int gear = -1;
};

/// Runs `steps` application frames, emitting the physics, the synthesizer input
/// and every rendered audio sample.
static void traceFrames(
    PhysicsSimulator *simulator,
    Engine *engine,
    int stepCount,
    const TraceOptions &options = TraceOptions()) {
    Crankshaft *crank = engine->getCrankshaft(0);
    Piston *piston = engine->getPiston(0);
    ConnectingRod *rod = engine->getConnectingRod(0);
    CylinderHead *head = engine->getHead(0);
    Intake *intake = engine->getIntake(0);
    ExhaustSystem *exhaust = engine->getExhaustSystem(0);
    CombustionChamber *chamber = engine->getChamber(0);

    const double timestep = simulator->getTimestep();

    for (int step = 0; step < stepCount; ++step) {
        if (step == 0 && options.gear >= 0) {
            simulator->getTransmission()->changeGear(options.gear - 1);
        }

        if (options.throttle >= 0.0) {
            engine->setSpeedControl(options.throttle);
        }

        if (options.clutch >= 0.0) {
            simulator->getTransmission()->setClutchPressure(options.clutch);
        }

        if (options.ignition >= 0) {
            engine->getIgnitionModule()->m_enabled = options.ignition != 0;
        }

        if (options.starter_seconds >= 0.0 &&
            step * timestep >= options.starter_seconds) {
            simulator->m_starterMotor.m_enabled = false;
        }

        if (options.dyno_rpm >= 0.0) {
            simulator->m_starterMotor.m_enabled = false;
            simulator->m_dyno.m_enabled = true;
            simulator->m_dyno.m_hold = true;
            simulator->m_dyno.m_rotationSpeed = units::rpm(options.dyno_rpm);
        }

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
