//! Port of `include/synthesizer.h` / `src/synthesizer.cpp`.
//!
//! This is where the engine's exhaust-flow signal becomes audio: the 10 kHz
//! simulation signal is resampled to 44.1 kHz through a Butterworth
//! anti-aliasing filter, dithered with a jitter filter, differentiated and
//! mixed with filtered white noise, convolved with the impulse response of the
//! exhaust system, low-passed, level-matched and clipped to `int16`.
//!
//! Differences from the original, none of which change a sample:
//!
//! * There is no audio rendering thread. `render_block()` is the body of
//!   `Synthesizer::renderAudio()` minus its condition-variable wait, so a
//!   caller controls exactly when blocks are produced (deterministic, and no
//!   lock ordering to reason about).
//! * `rand()` is replaced by the shared [`Rng`], matching the reference build
//!   where the physics and the DSP draw from the same global sequence.
//!
//! Everything else is transliterated: including the reference's use of
//! `m_filters->airNoiseLowPass` (always channel 0) regardless of the channel
//! being rendered, and its consumption of one `rand()` draw per sample for a
//! value (`r_0`) that is never used.

use crate::dsp::{
    ButterworthLowPassFilterF32, ButterworthLowPassFilterF64, ConvolutionFilter, DelayFilter,
    DerivativeFilter, JitterFilter, LevelingFilter, LowPassFilter, RingBuffer,
};
use crate::fuel::Rng;

/// `Synthesizer::AudioParameters`
#[derive(Clone, Copy, Debug)]
pub struct AudioParameters {
    pub volume: f32,
    pub convolution: f32,
    pub df_f_mix: f32,
    pub input_sample_noise: f32,
    pub input_sample_noise_frequency_cutoff: f32,
    pub air_noise: f32,
    pub air_noise_frequency_cutoff: f32,
    pub leveler_target: f32,
    pub leveler_max_gain: f32,
    pub leveler_min_gain: f32,
}

impl Default for AudioParameters {
    fn default() -> Self {
        Self {
            volume: 1.0,
            convolution: 1.0,
            df_f_mix: 0.01,
            input_sample_noise: 0.5,
            input_sample_noise_frequency_cutoff: 10000.0,
            air_noise: 1.0,
            air_noise_frequency_cutoff: 2000.0,
            leveler_target: 30000.0,
            leveler_max_gain: 1.9,
            leveler_min_gain: 0.00001,
        }
    }
}

/// `Synthesizer::Parameters`
#[derive(Clone, Copy, Debug)]
pub struct SynthesizerParameters {
    pub input_channel_count: usize,
    pub input_buffer_size: usize,
    pub audio_buffer_size: usize,
    pub input_sample_rate: f32,
    pub audio_sample_rate: f32,
    pub initial_audio_parameters: AudioParameters,
}

impl Default for SynthesizerParameters {
    fn default() -> Self {
        Self {
            input_channel_count: 1,
            input_buffer_size: 1024,
            audio_buffer_size: 44100,
            input_sample_rate: 10000.0,
            audio_sample_rate: 44100.0,
            initial_audio_parameters: AudioParameters::default(),
        }
    }
}

/// `Synthesizer::InputChannel`
#[derive(Clone, Debug, Default)]
pub struct InputChannel {
    pub data: RingBuffer<f32>,
    pub transfer_buffer: Vec<f32>,
    pub last_input_sample: f64,
}

/// `Synthesizer::ProcessingFilters`
#[derive(Clone, Debug, Default)]
pub struct ProcessingFilters {
    pub convolution: ConvolutionFilter,
    pub derivative: DerivativeFilter,
    pub jitter_filter: JitterFilter,
    pub air_noise_low_pass: ButterworthLowPassFilterF32,
    pub input_dc_filter: LowPassFilter,
    pub antialiasing: ButterworthLowPassFilterF64,
}

/// `Synthesizer`
#[derive(Clone, Debug)]
pub struct Synthesizer {
    /// `m_antialiasing` (a member, distinct from the per-channel filter).
    pub antialiasing: ButterworthLowPassFilterF32,
    pub leveling_filter: LevelingFilter,

    pub input_channels: Vec<InputChannel>,
    pub audio_parameters: AudioParameters,
    pub input_channel_count: usize,
    pub input_buffer_size: usize,
    pub input_samples_read: usize,
    pub latency: usize,
    pub input_write_offset: f64,
    pub last_input_sample_offset: f64,

    pub audio_buffer: RingBuffer<i16>,
    pub audio_buffer_size: usize,

    pub input_sample_rate: f32,
    pub audio_sample_rate: f32,

    pub processed: bool,

    pub filters: Vec<ProcessingFilters>,
}

impl Synthesizer {
    pub fn new() -> Self {
        Self {
            antialiasing: ButterworthLowPassFilterF32::new(),
            leveling_filter: LevelingFilter::new(),
            input_channels: Vec::new(),
            audio_parameters: AudioParameters::default(),
            input_channel_count: 0,
            input_buffer_size: 0,
            input_samples_read: 0,
            latency: 0,
            input_write_offset: 0.0,
            last_input_sample_offset: 0.0,
            audio_buffer: RingBuffer::new(),
            audio_buffer_size: 0,
            input_sample_rate: 0.0,
            audio_sample_rate: 0.0,
            processed: true,
            filters: Vec::new(),
        }
    }

    /// `Synthesizer::initialize`
    pub fn initialize(&mut self, p: &SynthesizerParameters) {
        self.input_channel_count = p.input_channel_count;
        self.input_buffer_size = p.input_buffer_size;
        self.input_write_offset = p.input_buffer_size as f64;
        self.audio_buffer_size = p.audio_buffer_size;
        self.input_sample_rate = p.input_sample_rate;
        self.audio_sample_rate = p.audio_sample_rate;
        self.audio_parameters = p.initial_audio_parameters;

        self.input_samples_read = 0;

        self.input_write_offset = 0.0;
        self.processed = true;

        self.audio_buffer.initialize(p.audio_buffer_size);

        self.input_channels = Vec::with_capacity(p.input_channel_count);
        for _ in 0..p.input_channel_count {
            let mut channel = InputChannel::default();
            channel.transfer_buffer = vec![0.0; p.input_buffer_size];
            channel.data.initialize(p.input_buffer_size);
            self.input_channels.push(channel);
        }

        self.filters = vec![ProcessingFilters::default(); p.input_channel_count];
        for filters in self.filters.iter_mut() {
            filters
                .air_noise_low_pass
                .set_cutoff_frequency(self.audio_parameters.air_noise_frequency_cutoff, self.audio_sample_rate);

            filters.derivative.dt = (1.0 / self.audio_sample_rate) as f32;

            filters.input_dc_filter.set_cutoff_frequency(10.0);
            filters.input_dc_filter.dt = (1.0 / self.audio_sample_rate) as f32;

            filters.jitter_filter.initialize(
                10,
                self.audio_parameters.input_sample_noise_frequency_cutoff,
                self.audio_sample_rate,
            );

            filters
                .antialiasing
                .set_cutoff_frequency(1900.0, self.audio_sample_rate as f64);
        }

        self.leveling_filter.target = self.audio_parameters.leveler_target;
        self.leveling_filter.max_level = self.audio_parameters.leveler_max_gain;
        self.leveling_filter.min_level = self.audio_parameters.leveler_min_gain;
        self.antialiasing.set_cutoff_frequency(
            self.audio_sample_rate as f32 * 0.45,
            self.audio_sample_rate as f32,
        );

        for _ in 0..self.audio_buffer_size {
            self.audio_buffer.write(0);
        }
    }

    /// `Synthesizer::initializeImpulseResponse`
    /// True once a non-empty impulse response has been installed with
    /// [`Synthesizer::initialize_impulse_response`].
    ///
    /// The reference stores the recording from the application and would read
    /// uninitialised memory if asked to render before that; the port refuses.
    pub fn has_impulse_response(&self) -> bool {
        !self.filters.is_empty() && self.filters[0].convolution.sample_count() > 0
    }

    pub fn initialize_impulse_response(
        &mut self,
        impulse_response: &[i16],
        volume: f32,
        index: usize,
    ) {
        let mut clipped_length = 0usize;
        for (i, &sample) in impulse_response.iter().enumerate() {
            if (sample as i32).abs() > 100 {
                clipped_length = i + 1;
            }
        }

        let sample_count = clipped_length.min(10000);
        self.filters[index].convolution.initialize(sample_count);
        let response = self.filters[index].convolution.impulse_response_mut();
        for i in 0..sample_count {
            // `volume * impulseResponse[i] / INT16_MAX` — the multiplication
            // happens before the division in C++.
            response[i] = volume * impulse_response[i] as f32 / i16::MAX as f32;
        }
    }

    /// `Synthesizer::readAudioOutput` — reads up to `samples`, zero-filling the
    /// tail, and returns how many real samples were produced.
    pub fn read_audio_output(&mut self, buffer: &mut [i16]) -> usize {
        let samples = buffer.len();
        let new_data_length = self.audio_buffer.size();
        let consumed = samples.min(new_data_length);

        self.audio_buffer.read_and_remove(consumed, buffer);

        for value in buffer.iter_mut().skip(consumed) {
            *value = 0;
        }

        consumed
    }

    /// `Synthesizer::writeInput`
    pub fn write_input(&mut self, data: &[f64]) {
        self.input_write_offset +=
            self.audio_sample_rate as f64 / self.input_sample_rate as f64;
        if self.input_write_offset >= self.input_buffer_size as f64 {
            self.input_write_offset -= self.input_buffer_size as f64;
        }

        for i in 0..self.input_channel_count {
            let last_input_sample = self.input_channels[i].last_input_sample;
            let base_index = self.input_channels[i].data.write_index();
            let distance =
                self.input_distance(self.input_write_offset, self.last_input_sample_offset);

            let mut s = self.input_distance(base_index as f64, self.last_input_sample_offset);
            while s <= distance {
                if s >= self.input_buffer_size as f64 {
                    s -= self.input_buffer_size as f64;
                }

                let f = s / distance;
                let sample = last_input_sample * (1.0 - f) + data[i] * f;

                // The reference narrows to float before the (double) filter and
                // the resulting f64 is stored in a float ring buffer.
                let filtered = self.filters[i].antialiasing.fast_f(sample as f32 as f64) as f32;
                self.input_channels[i].data.write(filtered);

                s += 1.0;
            }

            self.input_channels[i].last_input_sample = data[i];
        }

        self.last_input_sample_offset = self.input_write_offset;
    }

    /// `Synthesizer::endInputBlock`
    pub fn end_input_block(&mut self) {
        for channel in self.input_channels.iter_mut() {
            channel.data.remove_beginning(self.input_samples_read);
        }

        if self.input_channel_count != 0 {
            self.latency = self.input_channels[0].data.size();
        }

        self.input_samples_read = 0;
        self.processed = false;
    }

    pub fn get_latency(&self) -> f64 {
        self.latency as f64 / self.audio_sample_rate as f64
    }

    /// `Synthesizer::inputDistance`
    pub fn input_distance(&self, s1: f64, s0: f64) -> f64 {
        if s1 < s0 {
            self.input_buffer_size as f64 - s0 + s1
        } else {
            s1 - s0
        }
    }

    pub fn set_input_sample_rate(&mut self, sample_rate: f64) {
        if sample_rate != self.input_sample_rate as f64 {
            self.input_sample_rate = sample_rate as f32;
        }
    }

    /// Body of `Synthesizer::renderAudio()` without the condition-variable
    /// wait: pull one block of input, render it, return the block length.
    pub fn render_block(&mut self, rng: &mut Rng) -> usize {
        let available = self.audio_buffer.size();
        let n = (2000i64 - available as i64).max(0) as usize;
        let n = n.min(self.input_channels[0].data.size());

        // `RingBuffer::read` copies without consuming: the pending samples are
        // only removed by the next `endInputBlock`, exactly as in the original.
        for i in 0..self.input_channel_count {
            let block = self.input_channels[i].data.read_block(n);
            self.input_channels[i].transfer_buffer[..n].copy_from_slice(&block);
        }

        self.input_samples_read = n;
        self.processed = true;

        for i in 0..self.input_channel_count {
            self.filters[i].air_noise_low_pass.set_cutoff_frequency(
                self.audio_parameters.air_noise_frequency_cutoff,
                self.audio_sample_rate,
            );
            self.filters[i]
                .jitter_filter
                .set_jitter_scale(self.audio_parameters.input_sample_noise);
        }

        for i in 0..n {
            let sample = self.render_audio(i, rng);
            self.audio_buffer.write(sample);
        }

        n
    }

    /// `Synthesizer::renderAudio(int inputSample)`
    pub fn render_audio(&mut self, input_sample: usize, rng: &mut Rng) -> i16 {
        let air_noise = self.audio_parameters.air_noise;
        let df_f_mix = self.audio_parameters.df_f_mix;
        let conv_amount = self.audio_parameters.convolution;

        let mut signal = 0.0f32;
        for i in 0..self.input_channel_count {
            // Consumed but unused in the reference; the draw still advances the
            // shared sequence, so it must be kept.
            let _r_0 = (2.0 * rng.next_unit() - 1.0) as f32;

            let input = self.input_channels[i].transfer_buffer[input_sample];

            let jittered_sample = self.filters[i].jitter_filter.fast_f(input, 1.0);

            let f_in = jittered_sample;
            let f_dc = self.filters[i].input_dc_filter.fast_f(f_in);
            let f = f_in - f_dc;
            let f_p = self.filters[i].derivative.f(f_in);

            let noise = (2.0 * rng.next_unit() - 1.0) as f32;
            // Note: `m_filters->` in the reference — always channel 0.
            let r = self.filters[0].air_noise_low_pass.fast_f(noise);
            let r_mixed = air_noise * r + (1.0 - air_noise);

            let mut v_in = f_p * df_f_mix + f * r_mixed * (1.0 - df_f_mix);
            if v_in.is_subnormal() {
                v_in = 0.0;
            }

            let conv_out = self.filters[i].convolution.f(v_in);
            let v = conv_amount * conv_out + (1.0 - conv_amount) * v_in;

            signal += v;
        }

        signal = self.antialiasing.fast_f(signal);

        self.leveling_filter.target = self.audio_parameters.leveler_target;
        let leveled = self.leveling_filter.f(signal);
        let v_leveled = leveled * self.audio_parameters.volume;
        let mut r_int = v_leveled.round() as i32;
        if r_int > i16::MAX as i32 {
            r_int = i16::MAX as i32;
        } else if r_int < i16::MIN as i32 {
            r_int = i16::MIN as i32;
        }

        r_int as i16
    }

    pub fn get_leveler_gain(&self) -> f32 {
        self.leveling_filter.attenuation()
    }

    pub fn audio_parameters(&self) -> AudioParameters {
        self.audio_parameters
    }

    pub fn set_audio_parameters(&mut self, params: AudioParameters) {
        self.audio_parameters = params;
    }
}

impl Default for Synthesizer {
    fn default() -> Self {
        Self::new()
    }
}

/// `DelayFilter` for the exhaust path, kept here so the simulator can build the
/// per-cylinder delay from the header/pipe lengths.
pub type ExhaustDelayFilter = DelayFilter;

/// Speed of sound used by `PistonEngineSimulator::placeAndInitialize` to turn a
/// header/pipe length into a delay (`units::m` and `units::sec` are 1.0).
pub const SPEED_OF_SOUND: f64 = 343.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// An impulse response is mandatory: the reference indexes
    /// `m_shiftRegister` unconditionally, so an uninitialised convolution
    /// filter is undefined behaviour (a panic here).
    fn unit_impulse_response() -> Vec<i16> {
        let mut response = vec![0i16; 8];
        response[0] = i16::MAX;
        response
    }

    fn test_synth() -> Synthesizer {
        let mut synth = Synthesizer::new();
        synth.initialize(&SynthesizerParameters {
            input_channel_count: 1,
            input_buffer_size: 4096,
            audio_buffer_size: 44100,
            input_sample_rate: 10000.0,
            audio_sample_rate: 44100.0,
            initial_audio_parameters: AudioParameters::default(),
        });
        let response = unit_impulse_response();
        synth.initialize_impulse_response(&response, 1.0, 0);
        synth
    }

    #[test]
    fn silence_in_produces_silence_out_apart_from_dither() {
        let mut synth = test_synth();
        let mut rng = Rng::new(1);
        let mut peak = 0i16;

        for _ in 0..200 {
            synth.write_input(&[0.0]);
            synth.end_input_block();
            let n = synth.render_block(&mut rng);
            let mut buffer = vec![0i16; n];
            let read = synth.read_audio_output(&mut buffer);
            for &s in &buffer[..read] {
                peak = peak.max(s.abs());
            }
        }

        assert!(peak < 500, "unexpected output level for silence: {peak}");
    }

    #[test]
    fn render_block_consumes_the_pending_input() {
        let mut synth = test_synth();
        let mut rng = Rng::new(1);

        // 100 input samples at 10 kHz is ~441 samples at 44.1 kHz.
        for _ in 0..100 {
            synth.write_input(&[1.0]);
        }
        synth.end_input_block();

        let pending = synth.input_channels[0].data.size();
        assert!(pending > 0);

        let n = synth.render_block(&mut rng);
        assert_eq!(n, pending);
        assert_eq!(synth.audio_buffer.size(), n);

        let mut buffer = vec![0i16; n];
        let read = synth.read_audio_output(&mut buffer);
        assert_eq!(read, n);
    }

    #[test]
    fn latency_tracks_the_pending_input() {
        let mut synth = test_synth();
        for _ in 0..10 {
            synth.write_input(&[0.5]);
        }
        synth.end_input_block();
        assert!(synth.get_latency() > 0.0);

        let mut rng = Rng::new(1);
        synth.render_block(&mut rng);
        synth.read_audio_output(&mut vec![0i16; 4096]);
        synth.end_input_block();
        assert_eq!(synth.latency, 0);
    }

    /// Golden output from `tools/parity/dsp_reference.cpp`'s synthesizer case:
    /// one 1600-scaled sample per frame, the deterministic impulse response, and
    /// the default audio parameters.
    #[test]
    fn renders_the_reference_audio() {
        let mut synth = Synthesizer::new();
        synth.initialize(&SynthesizerParameters {
            input_channel_count: 1,
            input_buffer_size: 4096,
            audio_buffer_size: 44100,
            input_sample_rate: 10000.0,
            audio_sample_rate: 44100.0,
            initial_audio_parameters: AudioParameters::default(),
        });

        let response: Vec<i16> = (0..512)
            .map(|i| {
                let value = ((i as i32 * 7919) % 4001) - 2000;
                let envelope = 255 - (i as i32 % 256);
                (value * envelope / 256) as i16
            })
            .collect();
        synth.initialize_impulse_response(&response, 1.0, 0);

        const EXPECTED: [i16; 20] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 44, 423, 220, -1070, -914, -85, -1686, -2543, -517, -2744,
            -8277,
        ];

        let mut rng = Rng::new(1);
        let mut actual = Vec::new();
        let mut sample_index = 0usize;

        for _ in 0..5 {
            let input = ((sample_index as i64 * 7919) % 2001 - 1000) as f64 / 1000.0;
            sample_index += 1;
            synth.write_input(&[1600.0 * input]);
            synth.end_input_block();
            synth.render_block(&mut rng);

            let mut audio = vec![0i16; 4096];
            let produced = synth.read_audio_output(&mut audio);
            actual.extend_from_slice(&audio[..produced]);
        }

        assert_eq!(&actual[..20], &EXPECTED);
    }
}
