//! Port of the DSP chain used by the synthesizer:
//! `ring_buffer.h`, `filter.h`, `low_pass_filter.h`, `butterworth_low_pass_filter.h`,
//! `derivative_filter.h`, `delay_filter.h`, `convolution_filter.h`,
//! `leveling_filter.h`, `jitter_filter.h`.
//!
//! Everything here is shared between the C++ and Rust builds of the *audio*
//! path, so the same transliteration rules apply as for the physics core:
//! identical operation order, and identical intermediate precision. The C++
//! mixes `float` and `double` in several places (`0.999f`, `0.9 * m_attenuation
//! + 0.1 * attenuation`, the Butterworth coefficient formulas); the explicit
//! `as` conversions below reproduce exactly where the reference narrows.
//!
//! The jitter filter uses `std::default_random_engine` +
//! `std::uniform_real_distribution<float>`, so `MinStdRand0` below replicates
//! libstdc++'s `minstd_rand0` and its `generate_canonical` adaptor. Without
//! that, the dither noise (and therefore every output sample) would differ
//! between the two implementations for reasons that have nothing to do with the
//! port.

use crate::constants;

// ---------------------------------------------------------------------------
// ring_buffer.h
// ---------------------------------------------------------------------------

/// `RingBuffer<T_Data>`
#[derive(Clone, Debug)]
pub struct RingBuffer<T> {
    buffer: Vec<T>,
    capacity: usize,
    write_index: usize,
    start: usize,
}

impl<T: Copy + Default> RingBuffer<T> {
    /// `RingBuffer::RingBuffer()` — an uninitialized (zero-capacity) buffer.
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            capacity: 0,
            write_index: 0,
            start: 0,
        }
    }

    /// `RingBuffer::initialize`
    pub fn initialize(&mut self, capacity: usize) {
        self.buffer = vec![T::default(); capacity];
        self.capacity = capacity;
        self.write_index = 0;
        self.start = 0;
    }

    pub fn write(&mut self, data: T) {
        self.buffer[self.write_index] = data;

        self.write_index += 1;
        if self.write_index >= self.capacity {
            self.write_index = 0;
        }
    }

    /// `RingBuffer::read(size_t index)`
    pub fn read(&self, index: usize) -> T {
        if self.start + index >= self.capacity {
            self.buffer[self.start + index - self.capacity]
        } else {
            self.buffer[self.start + index]
        }
    }

    /// `RingBuffer::read(size_t n, T_Data *target)`
    pub fn read_block(&self, n: usize) -> Vec<T> {
        let mut target = Vec::with_capacity(n);
        if self.start + n < self.capacity {
            target.extend_from_slice(&self.buffer[self.start..self.start + n]);
        } else {
            let head = self.capacity - self.start;
            target.extend_from_slice(&self.buffer[self.start..self.start + head]);
            target.extend_from_slice(&self.buffer[0..n - head]);
        }
        target
    }

    /// `RingBuffer::read(size_t n, T_Data *target)` into a caller-provided
    /// slice (the C++ signature takes a raw pointer).
    pub fn read_into(&self, n: usize, target: &mut [T]) {
        if self.start + n < self.capacity {
            target[..n].copy_from_slice(&self.buffer[self.start..self.start + n]);
        } else {
            let head = self.capacity - self.start;
            target[..head].copy_from_slice(&self.buffer[self.start..self.start + head]);
            target[head..n].copy_from_slice(&self.buffer[0..n - head]);
        }
    }

    /// `RingBuffer::readAndRemove`
    pub fn read_and_remove(&mut self, n: usize, target: &mut [T]) {
        if self.start + n < self.capacity {
            target[..n].copy_from_slice(&self.buffer[self.start..self.start + n]);
        } else {
            let head = self.capacity - self.start;
            target[..head].copy_from_slice(&self.buffer[self.start..self.start + head]);
            target[head..n].copy_from_slice(&self.buffer[0..n - head]);
        }

        self.start += n;
        if self.start >= self.capacity {
            self.start -= self.capacity;
        }
    }

    pub fn set_write_index(&mut self, write_index: usize) {
        self.write_index = write_index;
    }

    /// `RingBuffer::removeBeginning`
    pub fn remove_beginning(&mut self, n: usize) {
        self.start += n;
        if self.start >= self.capacity {
            self.start -= self.capacity;
        }
    }

    pub fn set_start_index(&mut self, start_index: usize) {
        self.start = start_index;
    }

    pub fn size(&self) -> usize {
        if self.write_index < self.start {
            self.write_index + (self.capacity - self.start)
        } else {
            self.write_index - self.start
        }
    }

    pub fn write_index(&self) -> usize {
        self.write_index
    }

    pub fn start(&self) -> usize {
        self.start
    }
}

impl<T: Copy + Default> Default for RingBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// filter.h / low_pass_filter.h
// ---------------------------------------------------------------------------

/// `LowPassFilter`
#[derive(Clone, Debug)]
pub struct LowPassFilter {
    pub dt: f32,
    y: f32,
    rc: f32,
}

impl LowPassFilter {
    pub fn new() -> Self {
        Self {
            y: 0.0,
            rc: 0.0,
            dt: 1.0 / 44100.0,
        }
    }

    /// `LowPassFilter::fast_f`
    pub fn fast_f(&mut self, sample: f32) -> f32 {
        let alpha = self.dt / (self.rc + self.dt);
        self.y = alpha * sample + (1.0 - alpha) * self.y;

        self.y
    }

    /// `LowPassFilter::setCutoffFrequency`
    pub fn set_cutoff_frequency(&mut self, f: f32) {
        self.rc = 1.0 / (f * 2.0 * constants::PI as f32);
    }
}

impl Default for LowPassFilter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// butterworth_low_pass_filter.h
// ---------------------------------------------------------------------------

macro_rules! butterworth_filter {
    ($name:ident, $real:ty, $wide:ty) => {
        /// `ButterworthLowPassFilter<T_Real>` (4th order, two biquads in one
        /// direct-form-I stage, exactly as the reference writes it).
        #[derive(Clone, Debug)]
        pub struct $name {
            y: RingBuffer<$real>,
            x: RingBuffer<$real>,
            a: [$real; 5],
            f_4: $real,
        }

        impl $name {
            pub fn new() -> Self {
                let mut y = RingBuffer::new();
                y.initialize(4);

                let mut x = RingBuffer::new();
                x.initialize(4);

                for _ in 0..4 {
                    y.write(0.0);
                    x.write(0.0);
                }

                Self {
                    y,
                    x,
                    a: [0.0; 5],
                    f_4: 0.0,
                }
            }

            /// `ButterworthLowPassFilter::fast_f`
            /// The five denominator coefficients and `f_4` (exposed for the
            /// filter-level parity harness).
            pub fn coefficients(&self) -> ([$real; 5], $real) {
                (self.a, self.f_4)
            }

            pub fn fast_f(&mut self, sample: $real) -> $real {
                let y_prev = [self.y.read(3), self.y.read(2), self.y.read(1), self.y.read(0)];
                let x_prev = [self.x.read(3), self.x.read(2), self.x.read(1), self.x.read(0)];

                let n = self.f_4 / self.a[0]
                    * (sample
                        + 4.0 * x_prev[0]
                        + 6.0 * x_prev[1]
                        + 4.0 * x_prev[2]
                        + x_prev[3]);
                let d = -self.a[1] * y_prev[0]
                    - self.a[2] * y_prev[1]
                    - self.a[3] * y_prev[2]
                    - self.a[4] * y_prev[3];
                let y = n + d;

                self.x.remove_beginning(1);
                self.x.write(sample);

                self.y.remove_beginning(1);
                self.y.write(y);

                y
            }

            /// `ButterworthLowPassFilter::setCutoffFrequency`
            ///
            /// The reference mixes precisions here: `f`, `f_2`, `f_3`, `f_4` and
            /// `(m + n) * f` stay at the filter's own precision, while the `2.0`
            /// literals promote `(2.0 + n * m)` and the whole polynomial sum to
            /// `double`. Both are reproduced exactly; getting this wrong shifts
            /// every coefficient by ~1 ulp and the resulting noise floor by
            /// enough to change audio samples.
            pub fn set_cutoff_frequency(&mut self, f_c: $real, sample_rate: $real) {
                let f: $real = (constants::PI as $real * f_c / sample_rate).tan();
                let f_2: $real = f * f;
                let f_3: $real = f_2 * f;
                let f_4: $real = f_2 * f_2;

                let m: $real =
                    (-2.0 as $wide * (5.0 as $wide * constants::PI as $wide / 8.0).cos()) as $real;
                let n: $real =
                    (-2.0 as $wide * (7.0 as $wide * constants::PI as $wide / 8.0).cos()) as $real;

                let m_n_sum: $real = m + n;
                // `m * n` is a float multiply in the reference; only the `2.0 +`
                // promotes to double.
                let m_n: $real = m * n;
                let two_plus_mn: $wide = 2.0 as $wide + (m_n as $wide);

                let a0: $wide = 1.0 as $wide
                    + ((m_n_sum * f) as $wide)
                    + (two_plus_mn * (f_2 as $wide))
                    + ((m_n_sum * f_3) as $wide)
                    + (f_4 as $wide);
                self.a[0] = a0 as $real;

                let a1: $wide = (-4.0 as $wide
                    - 2.0 as $wide * ((n + m) as $wide) * (f as $wide)
                    + 2.0 as $wide * ((m + n) as $wide) * (f_3 as $wide)
                    + 4.0 as $wide * (f_4 as $wide))
                    / (self.a[0] as $wide);
                self.a[1] = a1 as $real;

                let a2: $wide = (6.0 as $wide
                    - 2.0 as $wide * two_plus_mn * (f_2 as $wide)
                    + 6.0 as $wide * (f_4 as $wide))
                    / (self.a[0] as $wide);
                self.a[2] = a2 as $real;

                let a3: $wide = (-4.0 as $wide
                    + 2.0 as $wide * ((m + n) as $wide) * (f as $wide)
                    - 2.0 as $wide * ((m + n) as $wide) * (f_3 as $wide)
                    + 4.0 as $wide * (f_4 as $wide))
                    / (self.a[0] as $wide);
                self.a[3] = a3 as $real;

                let a4: $wide = (1.0 as $wide
                    - ((n + m) * f) as $wide
                    + two_plus_mn * (f_2 as $wide)
                    - ((m + n) * f_3) as $wide
                    + (f_4 as $wide))
                    / (self.a[0] as $wide);
                self.a[4] = a4 as $real;

                self.f_4 = f_4;
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

butterworth_filter!(ButterworthLowPassFilterF32, f32, f64);
butterworth_filter!(ButterworthLowPassFilterF64, f64, f64);

// ---------------------------------------------------------------------------
// derivative_filter.h
// ---------------------------------------------------------------------------

/// `DerivativeFilter`
#[derive(Clone, Debug, Default)]
pub struct DerivativeFilter {
    pub dt: f32,
    previous: f32,
}

impl DerivativeFilter {
    pub fn new() -> Self {
        Self {
            previous: 0.0,
            dt: 0.0,
        }
    }

    pub fn f(&mut self, sample: f32) -> f32 {
        let temp = self.previous;
        self.previous = sample;

        (sample - temp) / self.dt
    }
}

// ---------------------------------------------------------------------------
// delay_filter.h
// ---------------------------------------------------------------------------

/// `DelayFilter` (used on the exhaust flow signal before it reaches the
/// synthesizer, so the pipe length shows up as a delay).
#[derive(Clone, Debug, Default)]
pub struct DelayFilter {
    latency_samples: i32,
    history: RingBuffer<f64>,
}

impl DelayFilter {
    pub fn new() -> Self {
        Self {
            latency_samples: 0,
            history: RingBuffer::new(),
        }
    }

    /// `DelayFilter::initialize`
    pub fn initialize(&mut self, delay: f64, audio_frequency: f64) {
        let samples = (delay * audio_frequency).round() as i32;
        let capacity = (samples + 32) as usize;

        self.history.initialize(capacity);
        self.latency_samples = samples;
    }

    /// `DelayFilter::fast_f`
    pub fn fast_f(&mut self, sample: f64) -> f64 {
        self.history.write(sample);

        if self.history.size() <= self.latency_samples as usize {
            0.0
        } else {
            let mut v = [0.0];
            self.history.read_and_remove(1, &mut v);
            v[0]
        }
    }
}

// ---------------------------------------------------------------------------
// convolution_filter.h
// ---------------------------------------------------------------------------

/// `ConvolutionFilter`
#[derive(Clone, Debug, Default)]
pub struct ConvolutionFilter {
    shift_register: Vec<f32>,
    shift_offset: usize,
    impulse_response: Vec<f32>,
    sample_count: usize,
}

impl ConvolutionFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `ConvolutionFilter::initialize`
    pub fn initialize(&mut self, samples: usize) {
        self.sample_count = samples;
        self.shift_offset = 0;
        self.shift_register = vec![0.0; samples];
        self.impulse_response = vec![0.0; samples];
    }

    /// `ConvolutionFilter::getImpulseResponse`
    pub fn impulse_response_mut(&mut self) -> &mut [f32] {
        &mut self.impulse_response
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    /// `ConvolutionFilter::f`
    pub fn f(&mut self, sample: f32) -> f32 {
        self.shift_register[self.shift_offset] = sample;

        let mut result = 0.0f32;
        for i in 0..self.sample_count - self.shift_offset {
            result += self.impulse_response[i] * self.shift_register[i + self.shift_offset];
        }

        for i in self.sample_count - self.shift_offset..self.sample_count {
            result += self.impulse_response[i]
                * self.shift_register[i - (self.sample_count - self.shift_offset)];
        }

        self.shift_offset = (self.shift_offset + self.sample_count - 1) % self.sample_count;

        result
    }

    /// Two consecutive samples of *this* filter in one pass.
    ///
    /// `f` is one dependent add per tap, so a 5 636-tap response costs 5 636 add
    /// latencies per sample, and a mono engine never reaches `f_pair` (which
    /// needs a second filter). Two consecutive output samples of the same filter
    /// are two independent chains:
    ///
    /// ```text
    /// out(s)   = sum h[i]   * R[(o+i) mod n]              i = 0 .. n-1
    /// out(s+1) = h[0] * y + sum h[i+1] * R[(o+i) mod n]   i = 0 .. n-2
    /// ```
    ///
    /// because `R[(o+k) mod n]` holds `x[s-k]` and `y` is `x[s+1]`. Every term is
    /// the same product the single-sample path would form, added in the same
    /// order -- `out(s+1)`'s leading `h[0]*y` is its first term, exactly as `f`
    /// would add it. Nothing is reassociated, so both results are bit-identical to
    /// two `f` calls; `paired_convolution_in_one_filter_is_bit_identical` proves
    /// it, down to `-0.0`.
    ///
    /// Measured at 1.99x `f` on a 5 636-tap response (three samples per pass buys
    /// another 4%, and two chains already saturate the machine's add throughput:
    /// `docs/performance-options.md` §2).
    pub fn f_two(&mut self, x: f32, y: f32) -> (f32, f32) {
        let n = self.sample_count;

        // Nothing to interleave: fall back to the one-sample path.
        if n < 2 || self.shift_offset == 0 {
            return (self.f(x), self.f(y));
        }

        let o = self.shift_offset;
        self.shift_register[o] = x;

        let h = &self.impulse_response;
        let r = &self.shift_register;
        let boundary = n - o;

        let mut first = 0.0f32;
        let mut second = h[0] * y;

        // Taps that sit before the wrap in the shift register ...
        for i in 0..boundary {
            let v = r[o + i];
            first += h[i] * v;
            second += h[i + 1] * v;
        }

        // ... then the ones after it. `first` gets the last tap on its own,
        // because `second` has already taken `y` as its newest sample.
        for i in boundary..n - 1 {
            let v = r[i - boundary];
            first += h[i] * v;
            second += h[i + 1] * v;
        }
        first += h[n - 1] * r[o - 1];

        self.shift_register[o - 1] = y;
        self.shift_offset = (o + n - 2) % n;

        (first, second)
    }

    /// Two `f()` calls in one pass, for the two channels of the synthesizer.
    ///
    /// `f` is a chain of `sample_count` dependent multiply-adds: nothing in it
    /// can start until the previous add has finished, so it costs about four
    /// cycles a tap however fast the machine is. At the 10 000 taps the engine
    /// simulator's impulse responses are clipped to, that is the entire cost of
    /// rendering a frame -- 44 100 samples a second, two channels, 20 000
    /// serial adds each. Two *independent* chains interleaved in one loop fill
    /// the pipeline instead of waiting on it.
    ///
    /// This is not a shortcut that trades accuracy for speed: each filter keeps
    /// its own shift register, its own offset and its own order of summation, so
    /// the pair of results is bit-identical to calling `f` twice. Reordering the
    /// additions -- a wider accumulator, a partitioned FFT, four partial sums --
    /// would not be.
    pub fn f_pair(a: &mut Self, x: f32, b: &mut Self, y: f32) -> (f32, f32) {
        a.shift_register[a.shift_offset] = x;
        b.shift_register[b.shift_offset] = y;

        let (na, oa) = (a.sample_count, a.shift_offset);
        let (nb, ob) = (b.sample_count, b.shift_offset);
        let (la, lb) = (na - oa, nb - ob);

        let mut result_a = 0.0f32;
        let mut result_b = 0.0f32;

        if na == nb && oa == ob {
            // The usual case: both channels were initialised together and have
            // been stepped together, so the registers wrap at the same index and
            // the two chains can share the loop bounds exactly.
            let shift_a = &a.shift_register;
            let shift_b = &b.shift_register;
            let response_a = &a.impulse_response;
            let response_b = &b.impulse_response;

            for i in 0..la {
                result_a += response_a[i] * shift_a[i + oa];
                result_b += response_b[i] * shift_b[i + oa];
            }

            for i in la..na {
                let j = i - la;
                result_a += response_a[i] * shift_a[j];
                result_b += response_b[i] * shift_b[j];
            }
        } else {
            // Different lengths or offsets: run the tails separately. Still
            // interleaved over the overlap, and each chain still in its own order.
            let overlap = la.min(lb);
            for i in 0..overlap {
                result_a += a.impulse_response[i] * a.shift_register[i + oa];
                result_b += b.impulse_response[i] * b.shift_register[i + ob];
            }

            for i in overlap..la {
                result_a += a.impulse_response[i] * a.shift_register[i + oa];
            }

            for i in overlap..lb {
                result_b += b.impulse_response[i] * b.shift_register[i + ob];
            }

            if la < na {
                for i in la..na {
                    let j = i - la;
                    result_a += a.impulse_response[i] * a.shift_register[j];
                }
            }

            if lb < nb {
                for i in lb..nb {
                    let j = i - lb;
                    result_b += b.impulse_response[i] * b.shift_register[j];
                }
            }
        }

        a.shift_offset = (oa + na - 1) % na;
        b.shift_offset = (ob + nb - 1) % nb;

        (result_a, result_b)
    }
}

// ---------------------------------------------------------------------------
// leveling_filter.h
// ---------------------------------------------------------------------------

/// `LevelingFilter`
#[derive(Clone, Debug)]
pub struct LevelingFilter {
    peak: f32,
    attenuation: f32,

    pub max_level: f32,
    pub min_level: f32,
    pub target: f32,
}

impl LevelingFilter {
    pub fn new() -> Self {
        Self {
            peak: 30000.0,
            attenuation: 1.0,
            target: 30000.0,
            min_level: 0.0,
            max_level: 1.0,
        }
    }

    pub fn attenuation(&self) -> f32 {
        self.attenuation
    }

    /// `LevelingFilter::f`
    pub fn f(&mut self, sample: f32) -> f32 {
        self.peak = 0.999f32 * self.peak;
        if sample.abs() > self.peak {
            self.peak = sample.abs();
        }

        if self.peak == 0.0 {
            return 0.0;
        }

        let raw_attenuation = self.target / self.peak;

        let mut attenuation = raw_attenuation;
        if attenuation < self.min_level {
            attenuation = self.min_level;
        } else if attenuation > self.max_level {
            attenuation = self.max_level;
        }

        // `m_attenuation = 0.9 * m_attenuation + 0.1 * attenuation` in C++
        // promotes both operands to double before narrowing back to float.
        self.attenuation = (0.9f64 * self.attenuation as f64 + 0.1f64 * attenuation as f64) as f32;

        sample * self.attenuation
    }
}

impl Default for LevelingFilter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// libstdc++ <random> compatibility (jitter filter dither noise)
// ---------------------------------------------------------------------------

/// libstdc++'s `std::default_random_engine` is `std::minstd_rand0`
/// (`linear_congruential_engine<uint_fast32_t, 16807, 0, 2147483647>`).
#[derive(Clone, Debug)]
pub struct MinStdRand0 {
    state: u32,
}

impl MinStdRand0 {
    /// Default-constructed `std::default_random_engine` (seed 1).
    pub fn new() -> Self {
        Self { state: 1 }
    }

    pub fn next_u32(&mut self) -> u32 {
        self.state = ((16807u64 * self.state as u64) % 2147483647) as u32;
        self.state
    }

    /// `std::uniform_real_distribution<float>(0.0f, b)(engine)` as implemented
    /// by libstdc++: `generate_canonical<float, 24>` through `_Adaptor`, with
    /// `__m = 1` because the engine supplies 31 bits per draw.
    pub fn uniform_real_0_b(&mut self, b: f32) -> f32 {
        let u = self.next_u32();
        // generate_canonical<float, 24>: sum = float(u - min) * 1; tmp = 1 * r
        // with r = max - min + 1 = 2^31 in float (~2147483648.0f).
        let r: f32 = 2147483648.0;
        let mut ret = (u - 1) as f32 / r;
        if ret == 0.0 {
            ret = f32::MIN_POSITIVE;
        }

        ret * b + 0.0
    }
}

impl Default for MinStdRand0 {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// jitter_filter.h
// ---------------------------------------------------------------------------

fn clamp_f32(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `JitterFilter` — resamples the input with a small, noise-driven time offset,
/// which is what gives the synth its "air".
#[derive(Clone, Debug)]
pub struct JitterFilter {
    noise_filter: ButterworthLowPassFilterF32,
    jitter_scale: f32,
    max_jitter: i32,
    offset: i32,
    history: Vec<f32>,
    generator: MinStdRand0,
}

impl JitterFilter {
    pub fn new() -> Self {
        Self {
            noise_filter: ButterworthLowPassFilterF32::new(),
            jitter_scale: 0.0,
            max_jitter: 0,
            offset: 0,
            history: Vec::new(),
            generator: MinStdRand0::new(),
        }
    }

    /// `JitterFilter::initialize`
    pub fn initialize(&mut self, max_jitter: i32, cutoff_frequency: f32, audio_frequency: f32) {
        self.max_jitter = max_jitter;
        self.history = vec![0.0; max_jitter as usize];
        self.offset = 0;
        self.noise_filter
            .set_cutoff_frequency(cutoff_frequency, audio_frequency);
    }

    /// `JitterFilter::fast_f`
    pub fn fast_f(&mut self, sample: f32, jitter_scale: f32) -> f32 {
        self.history[self.offset as usize] = sample;
        self.offset += 1;

        if self.offset >= self.max_jitter {
            self.offset = 0;
        }

        let dist = self.generator.uniform_real_0_b((self.max_jitter - 1) as f32);
        let s = self.noise_filter.fast_f(dist * self.jitter_scale * jitter_scale);
        let s_i_0 = clamp_f32(s.floor(), 0.0, (self.max_jitter - 1) as f32);
        let s_i_1 = clamp_f32(s.ceil(), 0.0, (self.max_jitter - 1) as f32);

        let s_frac = s - s_i_0;

        let i_0 = s_i_0 as i32 + self.offset;
        let i_1 = s_i_1 as i32 + self.offset;

        let v0 = self.history[if i_0 >= self.max_jitter {
            (i_0 - self.max_jitter) as usize
        } else {
            i_0 as usize
        }];
        let v1 = self.history[if i_1 >= self.max_jitter {
            (i_1 - self.max_jitter) as usize
        } else {
            i_1 as usize
        }];

        v1 * s_frac + v0 * (1.0 - s_frac)
    }

    pub fn set_jitter_scale(&mut self, jitter_scale: f32) {
        self.jitter_scale = jitter_scale;
    }

    pub fn jitter_scale(&self) -> f32 {
        self.jitter_scale
    }
}

impl Default for JitterFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minstd_rand0_matches_reference_sequence() {
        // minstd_rand0 seeded with 1: x = 16807 * x mod (2^31 - 1).
        let mut rng = MinStdRand0::new();
        assert_eq!(rng.next_u32(), 16807);
        assert_eq!(rng.next_u32(), 282475249);
        assert_eq!(rng.next_u32(), 1622650073);
    }

    /// The reference computes `(2.0 + m * n) * f_2` with `m * n` as a *float*
    /// multiply and only the `2.0 +` promoted to double. Doing the product in
    /// double shifts every coefficient by an ulp, which is enough to change the
    /// noise floor and break bit-exact audio. These are the reference's bits.
    ///
    /// `black_box` keeps the cutoff runtime-valued: when LLVM can see a constant
    /// cutoff it folds `tan` at compile time, and its folded `tanf` differs from
    /// the `tanf` the reference calls at run time by an ulp.
    #[test]
    fn butterworth_coefficients_match_the_reference() {
        let cases: [(f32, [u32; 5]); 2] = [
            (1900.0, [0x3fb68b85, 0xc052cae7, 0x4083d5a2, 0xc01418c6, 0x3efbbd1f]),
            (10000.0, [0x4100b277, 0xbeb9f0a1, 0x3f071a3e, 0xbd9fc740, 0x3ca5d159]),
        ];

        for (cutoff, expected) in cases {
            let mut filter = ButterworthLowPassFilterF32::new();
            filter.set_cutoff_frequency(std::hint::black_box(cutoff), std::hint::black_box(44100.0));
            let (coefficients, _f_4) = filter.coefficients();

            for (i, (actual, expected)) in coefficients.iter().zip(expected).enumerate() {
                assert_eq!(
                    actual.to_bits(),
                    expected,
                    "a[{i}] at {cutoff} Hz: {actual:?} != {:?}",
                    f32::from_bits(expected)
                );
            }
        }
    }

    #[test]
    fn butterworth_attenuates_high_frequencies() {
        let mut low = ButterworthLowPassFilterF32::new();
        low.set_cutoff_frequency(2000.0, 44100.0);
        let mut low_sum = 0.0f32;

        let mut high = ButterworthLowPassFilterF32::new();
        high.set_cutoff_frequency(2000.0, 44100.0);
        let mut high_sum = 0.0f32;

        for i in 0..44100 {
            let t = i as f32 / 44100.0;
            let slow = (2.0 * std::f32::consts::PI * 50.0 * t).sin();
            let fast = (2.0 * std::f32::consts::PI * 10000.0 * t).sin();
            low_sum += low.fast_f(slow).abs();
            high_sum += high.fast_f(fast).abs();
        }

        assert!(low_sum > 100.0 * high_sum, "low={low_sum} high={high_sum}");
    }

    /// Characterisation: the reference's direct-form float IIR is numerically
    /// unstable when the cutoff is a very small fraction of the sample rate
    /// (it drifts past `f32::MAX` after ~30 000 samples at 100 Hz). The
    /// synthesizer only uses 1900 Hz / 2000 Hz / 0.45 * fs, all of which are
    /// well behaved, so this is recorded rather than "fixed" — changing the
    /// topology would change the sound.
    #[test]
    fn butterworth_diverges_at_pathological_cutoffs() {
        let mut filter = ButterworthLowPassFilterF32::new();
        filter.set_cutoff_frequency(100.0, 44100.0);

        let mut diverged = false;
        for i in 0..44100 {
            let t = i as f32 / 44100.0;
            let y = filter.fast_f((2.0 * std::f32::consts::PI * 50.0 * t).sin());
            if !y.is_finite() {
                diverged = true;
                break;
            }
        }

        assert!(diverged);
    }

    #[test]
    fn delay_filter_holds_zeros_for_the_latency() {
        let mut delay = DelayFilter::new();
        delay.initialize(0.01, 10000.0); // 100 samples
        for i in 0..100 {
            assert_eq!(delay.fast_f(i as f64), 0.0);
        }
        // The history is full now, so the oldest samples come back out.
        assert_eq!(delay.fast_f(100.0), 0.0);
        assert_eq!(delay.fast_f(101.0), 1.0);
        assert_eq!(delay.fast_f(102.0), 2.0);
    }

    #[test]
    fn convolution_with_unit_impulse_passes_the_signal_through() {
        let mut conv = ConvolutionFilter::new();
        conv.initialize(4);
        conv.impulse_response_mut()[0] = 1.0;

        for i in 0..10 {
            let x = i as f32;
            assert!((conv.f(x) - x).abs() < 1e-6);
        }
    }

    /// `f_pair` has to be two `f`s, bit for bit -- not approximately: the audio
    /// gate compares samples, and a single ulp here is a different waveform.
    #[test]
    fn paired_convolution_is_bit_identical_to_two_single_convolutions() {
        let mut seed = 0x2545f491_4f6cdd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32
        };

        // Same length, same offset: the interleaved fast path.
        let mut single_a = ConvolutionFilter::new();
        let mut single_b = ConvolutionFilter::new();
        single_a.initialize(37);
        single_b.initialize(37);
        let mut pair_a = single_a.clone();
        let mut pair_b = single_b.clone();

        for i in 0..37 {
            let value = next() / 1e6;
            single_a.impulse_response_mut()[i] = value;
            pair_a.impulse_response_mut()[i] = value;
            let value = next() / 1e7;
            single_b.impulse_response_mut()[i] = value;
            pair_b.impulse_response_mut()[i] = value;
        }

        for _ in 0..500 {
            let x = next() / 1e5;
            let y = next() / 1e5;
            let expected_a = single_a.f(x);
            let expected_b = single_b.f(y);
            let (actual_a, actual_b) = ConvolutionFilter::f_pair(&mut pair_a, x, &mut pair_b, y);

            assert_eq!(actual_a.to_bits(), expected_a.to_bits());
            assert_eq!(actual_b.to_bits(), expected_b.to_bits());
        }

        // Different lengths and therefore different wrapping points: the general
        // path, which must agree just as exactly.
        let mut single_a = ConvolutionFilter::new();
        let mut single_b = ConvolutionFilter::new();
        single_a.initialize(29);
        single_b.initialize(41);
        let mut pair_a = single_a.clone();
        let mut pair_b = single_b.clone();

        for i in 0..41 {
            let value = next() / 1e6;
            if i < 29 {
                single_a.impulse_response_mut()[i] = value;
                pair_a.impulse_response_mut()[i] = value;
            }
            single_b.impulse_response_mut()[i] = value;
            pair_b.impulse_response_mut()[i] = value;
        }

        for _ in 0..500 {
            let x = next() / 1e5;
            let y = next() / 1e5;
            let expected_a = single_a.f(x);
            let expected_b = single_b.f(y);
            let (actual_a, actual_b) = ConvolutionFilter::f_pair(&mut pair_a, x, &mut pair_b, y);

            assert_eq!(actual_a.to_bits(), expected_a.to_bits());
            assert_eq!(actual_b.to_bits(), expected_b.to_bits());
        }
    }

    /// `f_two` has to be two `f`s of the same filter, bit for bit, at every wrap
    /// position -- including on a response the length of the real one.
    #[test]
    fn paired_convolution_in_one_filter_is_bit_identical() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32
        };

        // Short responses first: every wrap phase of every length, so the two
        // boundaries in `f_two` are crossed many times.
        for n in 2..=40usize {
            let mut single = ConvolutionFilter::new();
            single.initialize(n);
            for i in 0..n {
                let value = next() / 1e6;
                single.impulse_response_mut()[i] = value;
            }
            let mut paired = single.clone();

            for _ in 0..(3 * n + 5) / 2 {
                let x = next() / 1e5;
                let y = next() / 1e5;

                let expected_x = single.f(x);
                let expected_y = single.f(y);
                let (actual_x, actual_y) = paired.f_two(x, y);

                assert_eq!(actual_x.to_bits(), expected_x.to_bits(), "n = {n}, first sample");
                assert_eq!(actual_y.to_bits(), expected_y.to_bits(), "n = {n}, second sample");
            }
        }

        // Then the response `assets/main.mr` installs, over more than three wrap
        // periods, which is the case the audio path actually runs.
        let n = 5636;
        let mut single = ConvolutionFilter::new();
        single.initialize(n);
        for i in 0..n {
            single.impulse_response_mut()[i] = ((i % 97) as f32 - 48.0) / 1000.0;
        }
        let mut paired = single.clone();

        for _ in 0..(2 * n) {
            let x = next() / 1e5;
            let y = next() / 1e5;

            let expected_x = single.f(x);
            let expected_y = single.f(y);
            let (actual_x, actual_y) = paired.f_two(x, y);

            assert_eq!(actual_x.to_bits(), expected_x.to_bits());
            assert_eq!(actual_y.to_bits(), expected_y.to_bits());
        }
    }

    #[test]
    fn leveling_filter_targets_the_configured_peak() {
        let mut leveler = LevelingFilter::new();
        leveler.target = 1000.0;
        leveler.max_level = 10.0;
        leveler.min_level = 0.00001;

        for _ in 0..10_000 {
            leveler.f(100.0);
        }

        let out = leveler.f(100.0);
        assert!(out.abs() > 0.0);
        assert!(leveler.attenuation() <= 10.0);
    }
}
