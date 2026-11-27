//! Prototype: two consecutive samples of *each* channel in one pass -- four
//! independent chains instead of `f_pair`'s two.
//!
//!     cargo run --release -p engine-sim-core --example pair_two_proto [taps] [samples]
//!
//! Bit-checks the prototype against one `f` call per sample per channel at every
//! wrap phase, then times it against the shipped `f_pair`.
use engine_sim_core::dsp::ConvolutionFilter;
use std::time::Instant;

#[derive(Clone)]
struct Filter {
    h: Vec<f32>,
    r: Vec<f32>,
    o: usize,
}

impl Filter {
    fn new(n: usize, seed: usize) -> Self {
        Filter {
            h: (0..n).map(|i| (((i * 31 + seed) % 97) as f32 - 48.0) / 1000.0).collect(),
            r: (0..n).map(|i| ((i % 61) as f32 - 30.0) / 400.0).collect(),
            o: 0,
        }
    }

    /// The shipped one-sample path, copied so the harness does not depend on it.
    fn f(&mut self, sample: f32) -> f32 {
        let n = self.h.len();
        self.r[self.o] = sample;
        let mut result = 0.0f32;
        for i in 0..n - self.o {
            result += self.h[i] * self.r[i + self.o];
        }
        for i in n - self.o..n {
            result += self.h[i] * self.r[i - (n - self.o)];
        }
        self.o = (self.o + n - 1) % n;
        result
    }
}

/// Two consecutive samples of each of two equal-length filters: the results are
/// `(a[s], a[s+1], b[s], b[s+1])`. Each output keeps `f`'s order -- the
/// `h[0] * y` leading term first, then the taps ascending -- so nothing is
/// reassociated.
#[allow(clippy::too_many_arguments)]
fn pair_two(
    ha: &[f32],
    ra: &mut [f32],
    ha1: f32,
    ya: f32,
    hb: &[f32],
    rb: &mut [f32],
    o: usize,
    xb: f32,
    yb: f32,
) -> (f32, f32, f32, f32) {
    let n = ha.len();
    let boundary = n - o;
    let (xa, ha0) = (ha1, ha[0]);
    ra[o] = xa;
    rb[o] = xb;

    let mut a0 = 0.0f32;
    let mut a1 = ha0 * ya;
    let mut b0 = 0.0f32;
    let mut b1 = hb[0] * yb;

    for i in 0..boundary.min(n - 1) {
        let va = ra[o + i];
        let vb = rb[o + i];
        a0 += ha[i] * va;
        a1 += ha[i + 1] * va;
        b0 += hb[i] * vb;
        b1 += hb[i + 1] * vb;
    }

    for i in boundary..n - 1 {
        let va = ra[i - boundary];
        let vb = rb[i - boundary];
        a0 += ha[i] * va;
        a1 += ha[i + 1] * va;
        b0 += hb[i] * vb;
        b1 += hb[i + 1] * vb;
    }

    // the last tap of each filter belongs to the first sample only
    let last = (n - 1 + n - boundary) % n;
    a0 += ha[n - 1] * ra[last];
    b0 += hb[n - 1] * rb[last];

    ra[o] = xa;
    rb[o] = xb;
    ra[(o + n - 1) % n] = ya;
    rb[(o + n - 1) % n] = yb;

    (a0, a1, b0, b1)
}

/// The same four chains, with the shift registers sliced so the per-tap
/// indexing carries no bounds checks.
#[allow(clippy::too_many_arguments)]
fn pair_two_fast(
    ha: &[f32],
    ra: &mut [f32],
    xa: f32,
    ya: f32,
    hb: &[f32],
    rb: &mut [f32],
    o: usize,
    xb: f32,
    yb: f32,
) -> (f32, f32, f32, f32) {
    let n = ha.len();
    let boundary = n - o;
    ra[o] = xa;
    rb[o] = xb;

    let mut a0 = 0.0f32;
    let mut a1 = ha[0] * ya;
    let mut b0 = 0.0f32;
    let mut b1 = hb[0] * yb;

    {
        let t1 = &ra[o..];
        let u1 = &rb[o..];
        for i in 0..boundary.min(n - 1) {
            let va = t1[i];
            let vb = u1[i];
            a0 += ha[i] * va;
            a1 += ha[i + 1] * va;
            b0 += hb[i] * vb;
            b1 += hb[i + 1] * vb;
        }
    }
    let last = if o > 0 { ra[o - 1] } else { ra[n - 1] };
    let last_b = if o > 0 { rb[o - 1] } else { rb[n - 1] };
    {
        let t2 = &ra[..o];
        let u2 = &rb[..o];
        for i in boundary..n - 1 {
            let va = t2[i - boundary];
            let vb = u2[i - boundary];
            a0 += ha[i] * va;
            a1 += ha[i + 1] * va;
            b0 += hb[i] * vb;
            b1 += hb[i + 1] * vb;
        }
    }
    a0 += ha[n - 1] * last;
    b0 += hb[n - 1] * last_b;

    ra[(o + n - 1) % n] = ya;
    rb[(o + n - 1) % n] = yb;

    (a0, a1, b0, b1)
}

fn step(o: usize, n: usize, by: usize) -> usize {
    (o + n - by) % n
}

fn main() {
    let mut checked = 0usize;
    let mut sizes: Vec<usize> = (2..40).collect();
    sizes.push(5636);
    for &n in &sizes {
        let phases: Vec<usize> = if n > 100 {
            vec![0, 1, n / 2, n - 1]
        } else {
            (0..n).collect()
        };
        for phase in phases {
            let mut a = Filter::new(n, 3);
            let mut b = Filter::new(n, 7);
            a.o = phase;
            b.o = phase;
            let mut pa = a.clone();
            let mut pb = b.clone();
            let mut fa = a.clone();
            let mut fb = b.clone();
            let steps = if n > 100 { 64 } else { 3 };
            for s in 0..steps {
                let x = ((s * 37 % 23) as f32 - 11.0) / 7.0;
                let y = ((s * 53 % 19) as f32 - 9.0) / 5.0;
                // what `f` produces, per channel, per sample
                let a0 = a.f(x);
                let b0 = b.f(y);
                let a1 = a.f(y);
                let b1 = b.f(x);

                let (p, q, r, u) = pair_two(&pa.h, &mut pa.r, x, y, &pb.h, &mut pb.r, pa.o, y, x);
                pa.o = step(pa.o, n, 2);
                pb.o = pa.o;

                // the fast form must agree with the plain one and with `f`
                {
                    let (fp, fq, fr, fu) =
                        pair_two_fast(&fa.h, &mut fa.r, x, y, &fb.h, &mut fb.r, fa.o, y, x);
                    fa.o = step(fa.o, n, 2);
                    fb.o = fa.o;
                    let same = fp.to_bits() == a0.to_bits()
                        && fq.to_bits() == a1.to_bits()
                        && fr.to_bits() == b0.to_bits()
                        && fu.to_bits() == b1.to_bits();
                    if !same {
                        println!("FAST MISMATCH n={n} phase={phase} step={s}: got {:?}", (fp, fq, fr, fu));
                        std::process::exit(1);
                    }
                }
                let ok = p.to_bits() == a0.to_bits()
                    && q.to_bits() == a1.to_bits()
                    && r.to_bits() == b0.to_bits()
                    && u.to_bits() == b1.to_bits();
                if !ok {
                    println!("MISMATCH n={n} phase={phase} step={s}: got {:?} want {:?}", (p, q, r, u), (a0, a1, b0, b1));
                    std::process::exit(1);
                }
                checked += 1;
            }
        }
    }
    println!("bit check: {checked} two-sample groups identical (every wrap phase of n = 2..39, plus 5 636)\n");

    let taps: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5636);
    let samples: usize = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(44100);
    println!("taps {taps} samples {samples}");

    let mut acc = 0.0f32;

    // shipped: one sample, two channels
    let mut a = ConvolutionFilter::new();
    let mut b = ConvolutionFilter::new();
    a.initialize(taps);
    b.initialize(taps);
    for i in 0..taps {
        let v = ((i % 97) as f32 - 48.0) / 1000.0;
        a.impulse_response_mut()[i] = v;
        b.impulse_response_mut()[i] = v;
    }
    let t = Instant::now();
    for _ in 0..samples {
        let (p, q) = ConvolutionFilter::f_pair(&mut a, 1.0, &mut b, -1.0);
        acc += p + q;
    }
    let pair = t.elapsed().as_secs_f64();

    // prototype: two samples, two channels
    let mut a2 = Filter::new(taps, 3);
    let mut b2 = Filter::new(taps, 7);
    let t = Instant::now();
    for _ in 0..samples / 2 {
        let (p, q, r, s) = pair_two(&a2.h, &mut a2.r, 1.0, -1.0, &b2.h, &mut b2.r, a2.o, -1.0, 1.0);
        acc += p + q + r + s;
        a2.o = step(a2.o, taps, 2);
        b2.o = a2.o;
    }
    let two = t.elapsed().as_secs_f64();

    let mut a4 = Filter::new(taps, 3);
    let mut b4 = Filter::new(taps, 7);
    let t = Instant::now();
    for _ in 0..samples / 2 {
        let (p, q, r, s) = pair_two_fast(&a4.h, &mut a4.r, 1.0, -1.0, &b4.h, &mut b4.r, a4.o, -1.0, 1.0);
        acc += p + q + r + s;
        a4.o = step(a4.o, taps, 2);
        b4.o = a4.o;
    }
    let fast = t.elapsed().as_secs_f64();

    let macs = (2 * taps * samples) as f64;
    println!("f_pair  (1 sample x 2 ch, 2 chains): {:.3} s  {:.2} G mac/s", pair, macs / pair / 1e9);
    println!("pair_two (2 samples x 2 ch, 4 chains): {:.3} s  {:.2} G mac/s", two, macs / two / 1e9);
    println!("pair_two_fast (sliced): {:.3} s  {:.2} G mac/s", fast, macs / fast / 1e9);
    println!("speed-up over f_pair: {:.2}x (plain {:.2}x)", pair / fast, pair / two);
    println!("[{acc}]");
}
