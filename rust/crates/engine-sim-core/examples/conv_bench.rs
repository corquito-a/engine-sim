//! What the convolution costs, on its own, away from the synthesizer.
//!
//!     cargo run --release -p engine-sim-core --example conv_bench [taps] [samples]
//!
//! The default is the application's case: a 5 636-tap impulse response (the one
//! `assets/main.mr` installs) and two seconds of 44.1 kHz audio. It prints one
//! chain and the paired form `render_audio` uses, so the difference
//! `ConvolutionFilter::f_pair` makes is a number rather than a claim -- about
//! 1.6x the multiply-adds per second, which is what two independent chains buy on
//! a machine whose add latency, not throughput, sets the pace.
use engine_sim_core::dsp::ConvolutionFilter;
use std::time::Instant;

fn main() {
    let taps: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5636);
    let samples: usize = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(88200);

    let mut a = ConvolutionFilter::new();
    let mut b = ConvolutionFilter::new();
    a.initialize(taps);
    b.initialize(taps);
    for i in 0..taps {
        let v = ((i % 97) as f32 - 48.0) / 1000.0;
        a.impulse_response_mut()[i] = v;
        b.impulse_response_mut()[i] = v;
    }

    let start = Instant::now();
    let mut acc = 0.0f32;
    let mut x = 1.0f32;
    for _ in 0..samples {
        acc += a.f(x);
        x = -x;
    }
    let single = start.elapsed();

    let mut a2 = ConvolutionFilter::new();
    let mut b2 = ConvolutionFilter::new();
    a2.initialize(taps);
    b2.initialize(taps);
    for i in 0..taps {
        let v = ((i % 97) as f32 - 48.0) / 1000.0;
        a2.impulse_response_mut()[i] = v;
        b2.impulse_response_mut()[i] = v;
    }
    let start = Instant::now();
    let mut x = 1.0f32;
    for _ in 0..samples {
        let (p, _q) = ConvolutionFilter::f_pair(&mut a2, x, &mut b2, x);
        acc += p;
        x = -x;
    }
    let paired = start.elapsed();

    let macs = (taps * samples) as f64;
    println!("taps {taps} samples {samples}");
    println!("one chain:  {:?}  ({:.2} G mac/s)", single, macs / single.as_secs_f64() / 1e9);
    println!("two chains: {:?}  ({:.2} G mac/s)", paired, 2.0 * macs / paired.as_secs_f64() / 1e9);
    println!("[{acc}]");
}
