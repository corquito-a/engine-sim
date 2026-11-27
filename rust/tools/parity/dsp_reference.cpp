// Filter-level parity harness: drives each DSP unit with the same input as
// `crates/engine-sim-core/examples/dsp_trace.rs` and prints the raw f64 bits of
// every output value.
//
// This exists to isolate a mismatch to a single filter instead of bisecting
// through the whole synthesizer. Filters are constructed exactly the way
// `Synthesizer::initialize()` constructs them.

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

#include "butterworth_low_pass_filter.h"
#include "constants.h"
#include "convolution_filter.h"
#include "delay_filter.h"
#include "derivative_filter.h"
#include "jitter_filter.h"
#include "leveling_filter.h"
#include "synthesizer.h"
#include "low_pass_filter.h"

static int s_step = 0;

/// The coefficients are protected; expose them so a mismatch can be attributed
/// to the coefficient calculation rather than the recursion.
template <typename T_Real>
class ProbeButterworth : public ButterworthLowPassFilter<T_Real> {
public:
    T_Real coefficient(int i) const { return this->m_a[i]; }
    T_Real f4() const { return this->m_f_4; }
};

static void emit(const char *name, double value) {
    unsigned long long bits;
    std::memcpy(&bits, &value, sizeof(bits));
    std::printf("%s,%016llx\n", name, bits);
}

/// Deterministic, libm-independent test signal in [-1, 1].
static double input_sample(int i) {
    return ((double)(((i * 7919) % 2001) - 1000)) / 1000.0;
}

static void build_impulse_response(int16_t *ir, int samples) {
    for (int i = 0; i < samples; ++i) {
        const int value = ((i * 7919) % 4001) - 2000;
        const int envelope = 255 - (i % 256);
        ir[i] = (int16_t)(value * envelope / 256);
    }
}

int main() {
    const int n = 2000;

    // ---------------------------------------------------- low pass (dc filter)
    {
        LowPassFilter f;
        f.setCutoffFrequency(10.0f);
        f.m_dt = 1 / 44100.0f;
        for (int i = 0; i < n; ++i) {
            emit("lowpass", (double)f.fast_f((float)input_sample(i)));
        }
    }

    // ------------------------------------------------------- butterworth f32
    {
        ButterworthLowPassFilter<float> f;
        f.setCutoffFrequency(1900.0f, 44100.0f);
        for (int i = 0; i < n; ++i) {
            emit("bw_f32_1900", (double)f.fast_f((float)input_sample(i)));
        }
    }
    for (float cutoff : { 100.0f, 1900.0f, 2000.0f, 10000.0f, 19845.0f }) {
        ProbeButterworth<float> probe;
        probe.setCutoffFrequency(cutoff, 44100.0f);
        char name[32];
        for (int i = 0; i < 5; ++i) {
            std::snprintf(name, sizeof(name), "bwcoef_a%d", i);
            emit(name, (double)probe.coefficient(i));
        }
        emit("bwcoef_f4", (double)probe.f4());
    }

    {
        ButterworthLowPassFilter<float> f;
        // volatile: keep `tanf` a run-time call, as it is in production, so the
        // gate does not encode this compiler's constant-folded `tanf`.
        volatile float cutoff = 10000.0f;
        f.setCutoffFrequency((float)cutoff, 44100.0f);
        for (int i = 0; i < n; ++i) {
            emit("bw_f32_10000", (double)f.fast_f((float)input_sample(i)));
        }
    }
    {
        ButterworthLowPassFilter<float> f;
        f.setCutoffFrequency(44100.0f * 0.45f, 44100.0f);
        for (int i = 0; i < n; ++i) {
            emit("bw_f32_045", (double)f.fast_f((float)input_sample(i)));
        }
    }

    // ------------------------------------------------------- butterworth f64
    {
        ButterworthLowPassFilter<double> f;
        f.setCutoffFrequency(1900.0f, (double)44100.0f);
        for (int i = 0; i < n; ++i) {
            emit("bw_f64_1900", f.fast_f((double)(float)input_sample(i)));
        }
    }

    // ------------------------------------------------------------- derivative
    {
        DerivativeFilter f;
        f.m_dt = 1 / 44100.0f;
        for (int i = 0; i < n; ++i) {
            emit("derivative", (double)f.f((float)input_sample(i)));
        }
    }

    // ------------------------------------------------------------------ delay
    {
        DelayFilter f;
        f.initialize(0.001, 10000.0);
        for (int i = 0; i < n; ++i) {
            emit("delay", f.fast_f(input_sample(i)));
        }
    }

    // ------------------------------------------------------------ convolution
    {
        int16_t ir[512];
        build_impulse_response(ir, 512);
        ConvolutionFilter f;
        f.initialize(512);
        for (int i = 0; i < 512; ++i) {
            f.getImpulseResponse()[i] = 1.0f * (float)ir[i] / INT16_MAX;
        }
        for (int i = 0; i < n; ++i) {
            emit("convolution", (double)f.f((float)input_sample(i)));
        }
        f.destroy(); // the reference destructor asserts on owned buffers
    }

    // ---------------------------------------------------------------- leveler
    {
        LevelingFilter f;
        f.p_target = 30000.0f;
        f.p_maxLevel = 1.9f;
        f.p_minLevel = 0.00001f;
        for (int i = 0; i < n; ++i) {
            const float x = (float)(input_sample(i) * (double)(1 + (i % 97)));
            emit("leveler", (double)f.f(x));
        }
    }

    // ----------------------------------------------------------------- jitter
    {
        JitterFilter f;
        f.initialize(10, 10000.0f, 44100.0f);
        f.setJitterScale(0.5f);
        for (int i = 0; i < n; ++i) {
            emit("jitter", (double)f.fast_f((float)input_sample(i)));
        }
    }

    // ----------------------------------------------------- <random> internals
    {
        std::default_random_engine generator;
        std::uniform_real_distribution<float> dist(0.0f, 9.0f);
        for (int i = 0; i < 20; ++i) {
            emit("uniform", (double)dist(generator));
        }
        emit("engine", (double)(generator() - std::default_random_engine::min()));
    }

    // ------------------------------------------------------- full synthesizer
    {
        Synthesizer *synth = new Synthesizer;
        Synthesizer::Parameters params;
        params.inputChannelCount = 1;
        params.inputBufferSize = 4096;
        params.audioBufferSize = 44100;
        params.inputSampleRate = 10000.0f;
        params.audioSampleRate = 44100.0f;
        synth->initialize(params);

        int16_t ir[512];
        build_impulse_response(ir, 512);
        synth->initializeImpulseResponse(ir, 512, 1.0f, 0);

        int sampleIndex = 0;
        for (int frame = 0; frame < 300; ++frame) {
            for (int k = 0; k < 1; ++k) {
                const double value = 1600.0 * input_sample(sampleIndex++);
                synth->writeInput(&value);
            }
            synth->endInputBlock();

            const int n = std::min(
                std::max(0, 2000 - (int)synth->m_audioBuffer.size()),
                (int)synth->m_inputChannels[0].data.size());
            for (int i = 0; i < synth->m_inputChannelCount; ++i) {
                synth->m_inputChannels[i].data.read(
                    (size_t)n, synth->m_inputChannels[i].transferBuffer);
            }
            synth->m_inputSamplesRead = n;
            synth->m_processed = true;
            for (int i = 0; i < synth->m_inputChannelCount; ++i) {
                synth->m_filters[i].airNoiseLowPass.setCutoffFrequency(
                    (float)synth->m_audioParameters.airNoiseFrequencyCutoff,
                    synth->m_audioSampleRate);
                synth->m_filters[i].jitterFilter.setJitterScale(
                    synth->m_audioParameters.inputSampleNoise);
            }
            for (int i = 0; i < n; ++i) {
                synth->m_audioBuffer.write(synth->renderAudio(i));
            }

            int16_t audio[4096];
            const int produced = synth->readAudioOutput(4096, audio);
            for (int i = 0; i < produced; ++i) {
                emit("synth_audio", (double)audio[i]);
            }
        }
    }

    return 0;
}
