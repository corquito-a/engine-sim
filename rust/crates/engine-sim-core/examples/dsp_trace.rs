//! Rust half of the filter-level parity check.
//!
//! Mirrors `rust/tools/parity/dsp_reference.cpp` filter for filter, printing
//! `<case>,<16 hex digits of the f64>` so the two outputs can be diffed
//! bit-for-bit (`rust/tools/parity/run_dsp_parity.sh`).

use engine_sim_core::fuel::Rng;
use engine_sim_core::synth::{AudioParameters, Synthesizer, SynthesizerParameters};
use engine_sim_core::dsp::{
    ButterworthLowPassFilterF32, ButterworthLowPassFilterF64, ConvolutionFilter, DelayFilter,
    DerivativeFilter, JitterFilter, LevelingFilter, LowPassFilter, MinStdRand0,
};

fn emit(name: &str, value: f64) {
    println!("{name},{:016x}", value.to_bits());
}

/// Deterministic, libm-independent test signal in [-1, 1].
fn input_sample(i: usize) -> f64 {
    ((i as i64 * 7919) % 2001 - 1000) as f64 / 1000.0
}

fn build_impulse_response(samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let value = ((i as i32 * 7919) % 4001) - 2000;
            let envelope = 255 - (i as i32 % 256);
            (value * envelope / 256) as i16
        })
        .collect()
}

fn main() {
    let n = 2000usize;

    {
        let mut f = LowPassFilter::new();
        f.set_cutoff_frequency(10.0);
        f.dt = 1.0 / 44100.0;
        for i in 0..n {
            emit("lowpass", f.fast_f(input_sample(i) as f32) as f64);
        }
    }

    {
        let mut f = ButterworthLowPassFilterF32::new();
        f.set_cutoff_frequency(1900.0, 44100.0);
        for i in 0..n {
            emit("bw_f32_1900", f.fast_f(input_sample(i) as f32) as f64);
        }
    }

    for cutoff in [100.0f32, 1900.0, 2000.0, 10000.0, 19845.0] {
        let mut probe = ButterworthLowPassFilterF32::new();
        probe.set_cutoff_frequency(std::hint::black_box(cutoff), 44100.0);
        let (a, f4) = probe.coefficients();
        for (i, &value) in a.iter().enumerate() {
            emit(&format!("bwcoef_a{i}"), value as f64);
        }
        emit("bwcoef_f4", f4 as f64);
    }

    {
        let mut f = ButterworthLowPassFilterF32::new();
        // `black_box`: keep `tan` a run-time call, as it is in production, so the
        // gate does not encode this compiler's constant-folded `tan`.
        f.set_cutoff_frequency(std::hint::black_box(10000.0), 44100.0);
        for i in 0..n {
            emit("bw_f32_10000", f.fast_f(input_sample(i) as f32) as f64);
        }
    }

    {
        let mut f = ButterworthLowPassFilterF32::new();
        f.set_cutoff_frequency(44100.0 * 0.45, 44100.0);
        for i in 0..n {
            emit("bw_f32_045", f.fast_f(input_sample(i) as f32) as f64);
        }
    }

    {
        let mut f = ButterworthLowPassFilterF64::new();
        f.set_cutoff_frequency(1900.0, 44100.0);
        for i in 0..n {
            emit("bw_f64_1900", f.fast_f(input_sample(i) as f32 as f64));
        }
    }

    {
        let mut f = DerivativeFilter::new();
        f.dt = 1.0 / 44100.0;
        for i in 0..n {
            emit("derivative", f.f(input_sample(i) as f32) as f64);
        }
    }

    {
        let mut f = DelayFilter::new();
        f.initialize(0.001, 10000.0);
        for i in 0..n {
            emit("delay", f.fast_f(input_sample(i)));
        }
    }

    {
        let response = build_impulse_response(512);
        let mut f = ConvolutionFilter::new();
        f.initialize(512);
        for i in 0..512 {
            f.impulse_response_mut()[i] = 1.0 * response[i] as f32 / i16::MAX as f32;
        }
        for i in 0..n {
            emit("convolution", f.f(input_sample(i) as f32) as f64);
        }
    }

    {
        let mut f = LevelingFilter::new();
        f.target = 30000.0;
        f.max_level = 1.9;
        f.min_level = 0.00001;
        for i in 0..n {
            let x = input_sample(i) * (1 + (i % 97)) as f64;
            emit("leveler", f.f(x as f32) as f64);
        }
    }

    {
        let mut f = JitterFilter::new();
        f.initialize(10, 10000.0, 44100.0);
        f.set_jitter_scale(0.5);
        for i in 0..n {
            emit("jitter", f.fast_f(input_sample(i) as f32, 1.0) as f64);
        }
    }

    {
        let mut rng = MinStdRand0::new();
        for _ in 0..20 {
            emit("uniform", rng.uniform_real_0_b(9.0) as f64);
        }
        emit("engine", (rng.next_u32() - 1) as f64);
    }

    {
        let mut synth = Synthesizer::new();
        synth.initialize(&SynthesizerParameters {
            input_channel_count: 1,
            input_buffer_size: 4096,
            audio_buffer_size: 44100,
            input_sample_rate: 10000.0,
            audio_sample_rate: 44100.0,
            initial_audio_parameters: AudioParameters::default(),
        });

        let response = build_impulse_response(512);
        synth.initialize_impulse_response(&response, 1.0, 0);

        let mut sample_index = 0usize;
        let mut rng = Rng::new(1);

        for _frame in 0..300 {
            let value = 1600.0 * input_sample(sample_index);
            sample_index += 1;
            synth.write_input(&[value]);
            synth.end_input_block();
            synth.render_block(&mut rng);

            let mut audio = [0i16; 4096];
            let produced = synth.read_audio_output(&mut audio);
            for &sample in &audio[..produced] {
                emit("synth_audio", sample as f64);
            }
        }
    }
}
