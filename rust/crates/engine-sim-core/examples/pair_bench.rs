//! Why the convolution stops where it does: how many independent chains this
//! machine keeps busy, and whether the stereo path has headroom left.
//!
//!     cargo run --release -p engine-sim-core --example pair_bench [taps] [samples]
//!
//! The first three variants are the shipped functions. The rest are the *shape*
//! of a pass that renders K samples at once -- `acc[j] += h[i + j] * v` with `v`
//! loaded once per tap and `h` read at K consecutive positions, which is exactly
//! `f_two`'s inner loop -- run over a contiguous response so the loop is one
//! region. A real K-sample pass splits the loop once where the shift register
//! wraps; both regions have this shape, so the per-tap cost measured here is the
//! per-tap cost the real function pays.
use engine_sim_core::dsp::ConvolutionFilter;
use std::time::Instant;

fn fill(v: &mut [f32]) {
    for (i, s) in v.iter_mut().enumerate() {
        *s = ((i % 97) as f32 - 48.0) / 1000.0;
    }
}

/// K samples of one filter in one pass: one `v` load and K response loads per
/// tap, K independent accumulator chains.
fn multi<const K: usize>(h: &[f32], r: &[f32], x: f32) -> [f32; K] {
    let n = h.len();
    let mut acc = [0.0f32; K];
    let mut v = x;
    for i in 0..n - K {
        v = r[i];
        for j in 0..K {
            acc[j] += h[i + j] * v;
        }
    }
    acc
}

/// K samples of two filters in one pass: 2K independent chains, 2 + 2K loads a
/// tap.
fn multi2<const K: usize>(
    ha: &[f32],
    ra: &[f32],
    hb: &[f32],
    rb: &[f32],
    x: f32,
) -> ([f32; K], [f32; K]) {
    let n = ha.len();
    let mut acc = [0.0f32; K];
    let mut acc_b = [0.0f32; K];
    let mut va = x;
    let mut vb = x;
    for i in 0..n - K {
        va = ra[i];
        vb = rb[i];
        for j in 0..K {
            acc[j] += ha[i + j] * va;
            acc_b[j] += hb[i + j] * vb;
        }
    }
    (acc, acc_b)
}

/// `macs` counts one multiply-add per tap per output sample per channel;
/// `visits` counts one per tap per output sample, so the last column is the cost
/// of a tap whatever the number of channels multiplied into it.
fn report(name: &str, secs: f64, macs: f64, visits: f64) {
    println!(
        "{name:34} {:>7.3} s  {:>6.2} G mac/s  {:>5.2} ns/tap",
        secs,
        macs / secs / 1e9,
        1e9 * secs / visits
    );
}

fn main() {
    let taps: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5636);
    let samples: usize = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(44100);
    println!(
        "taps {taps}  samples {samples}  ({:.1} KB a response, {:.2} GHz)\n",
        4.0 * taps as f64 / 1024.0,
        2.60
    );

    let mut acc = 0.0f32;
    let mut x = 1.0f32;

    // --- the shipped functions -------------------------------------------------
    let mut f1 = ConvolutionFilter::new();
    f1.initialize(taps);
    fill(f1.impulse_response_mut());
    let t = Instant::now();
    for _ in 0..samples {
        acc += f1.f(x);
        x = -x;
    }
    report("f: one chain", t.elapsed().as_secs_f64(), (taps * samples) as f64, (taps * samples) as f64);

    let mut f2 = ConvolutionFilter::new();
    f2.initialize(taps);
    fill(f2.impulse_response_mut());
    let t = Instant::now();
    for _ in 0..samples / 2 {
        let (p, q) = f2.f_two(x, -x);
        acc += p + q;
    }
    report("f_two: 2 samples, 1 filter", t.elapsed().as_secs_f64(), (2 * taps * (samples / 2)) as f64, (taps * samples) as f64);

    let mut a = ConvolutionFilter::new();
    let mut b = ConvolutionFilter::new();
    a.initialize(taps);
    b.initialize(taps);
    fill(a.impulse_response_mut());
    fill(b.impulse_response_mut());
    let t = Instant::now();
    for _ in 0..samples {
        let (p, q) = ConvolutionFilter::f_pair(&mut a, x, &mut b, -x);
        acc += p + q;
    }
    report("f_pair: 1 sample, 2 channels", t.elapsed().as_secs_f64(), (2 * taps * samples) as f64, (taps * samples) as f64);

    // --- the shape of a K-sample pass ------------------------------------------
    let h: Vec<f32> = (0..taps).map(|i| ((i % 97) as f32 - 48.0) / 1000.0).collect();
    let r: Vec<f32> = (0..taps).map(|i| (i as f32 * 0.001) - 3.0).collect();

    macro_rules! shape1 {
        ($k:literal) => {{
            let t = Instant::now();
            for _ in 0..samples / $k {
                let o: [f32; $k] = multi::<$k>(&h, &r, x);
                acc += o[0];
            }
            report(concat!("shape: ", $k, " samples x 1 filter"), t.elapsed().as_secs_f64(), ($k * taps * (samples / $k)) as f64, (($k * taps * (samples / $k)) as f64) / $k as f64);
        }};
    }
    shape1!(2);
    shape1!(3);
    shape1!(4);

    macro_rules! shape2 {
        ($k:literal) => {{
            let t = Instant::now();
            for _ in 0..samples / $k {
                let (oa, ob) = multi2::<$k>(&h, &r, &h, &r, x);
                acc += oa[0] + ob[0];
            }
            report(concat!("shape: ", $k, " samples x 2 channels"), t.elapsed().as_secs_f64(), (2 * $k * taps * (samples / $k)) as f64, (($k * taps * (samples / $k)) as f64) / $k as f64);
        }};
    }
    shape2!(1);
    shape2!(2);

    println!("\n[{acc}]");
}
