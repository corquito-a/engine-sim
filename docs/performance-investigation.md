# Where the time goes, and which of it is safe to take back

Item 4 of [`next-steps.md`](next-steps.md): a thorough performance investigation
of the port, with one standing constraint — **nothing here may change a single
output bit**. A speed-up that changes a float is not a speed-up, it is a
fidelity bug with a smaller diff.

So this document is two things at once:

* a measurement of every phase the port spends time in, per engine and per frame;
* for each candidate optimisation, the *argument* that it cannot change the
  numbers, and the gate that would catch it if the argument is wrong.

Two of the candidates were prototyped and measured end to end (O1, O2). Both
produced bit-identical output on every check available in this environment. The
rest are estimates with the reasoning shown, so they can be argued with.

This document is the *map*; [`performance-options.md`](performance-options.md) is
the *decision*: every option for O1–O3, contested on pro / con / risk / outcome,
with the design that came out of it and the order to land the pieces in.

Numbers are from a 2-vCPU sandbox, `rustc 1.88.0` `--release`, single-threaded
(the simulator has no threads). Absolute times move ±10% between runs; ratios and
shares are stable to a few percent. Nothing here needs `perf`; the recipes are in
[§9](#9-how-to-re-measure).

---

## 1. Summary

| | |
|---|---|
| The port runs **1.28–1.34× real time** on `assets/main.mr`; the original C++ runs 1.13× on the same asset. | The gap is real but small, and it is not in the physics. |
| **~59% of a frame is the gas-dynamics fluid loop**, 12–30% is the rigid-body constraint solve, 18–34% is audio, 5% is `chamber.update`. | Physics, solver, DSP — in that order. There is no single silly hot spot. |
| The **front end is quadratic** in the number of expansions: 960 million `is_equal` calls for `radial_9`. | O1 fixes it: `expand_all` 3.61 s → **0.23 s**, setup 4.15 s → **0.74 s**, output bit-identical (21 scripts, IR + graph dumps, 184 tests). |
| The convolution is a **single dependent chain** for mono engines, and mono is what `main.mr` is. | O2 renders two samples per pass: **1.98×** on the convolution, bit-identical by construction and by test. |
| Everything else is worth 0.1–4%. | The physics is the cost, and the physics is the thing being asked for. |

Ranked, with the discount already applied:

| # | change | measured gain | risk to exactness |
|---|---|---|---|
| **O1** | bucket the expansion table by a structural hash of the context chain | `expand_all` **15×**, whole front end **5.6×** (`radial_9` 4.15 s → 0.74 s) | none observed: IR and graph dumps of all 21 scripts identical |
| **O2** | convolution: two output samples per pass (mono), four chains for stereo | **1.98×** convolution on mono, 1.25× on stereo ⇒ ~10% of a frame | none: same operands, same order, per-sample bits compared |
| **O3** | hoist `get_volume` out of the eight fluid sub-steps | 3–14% of the fluid loop ⇒ 2–4% of a frame | none: same value, same expression, one evaluation instead of eight |
| **O4** | four chains instead of two for *stereo* convolution | **unproven**: a single run showed 3.13 vs 2.50 G MAC/s, but mono two-sample chains reach 3.15 G MAC/s on their own (§6), so the stereo gap is more likely the 2-response stream pattern than the chain count. Measure before proposing. | layout or count — needs its own measurement |
| **O5** | `build_graph` (0.45 s for `radial_9` after O1) | not yet attributed | unknown — needs its own profile |
| **O6** | `chamber.update`, `sample_triangle`, dyno bookkeeping, the `Mutex` per step | 5% total, and the pieces are ≤ 100 ns | not worth the diff |

With O1–O3 (and not O4, which is now unproven), a `main.mr`-style mono run drops
from 1.53 s to ~1.30 s of frame work and setup from 0.60 s to 0.18 s — **~25% end
to end** — with no arithmetic touched. That is the honest ceiling for "free" speed
in this codebase.
Everything beyond it is a physics decision (fewer sub-steps, shorter impulse
response, `f32` in the gas path), and every one of those changes the output.

---

## 2. Method, and one correction to the earlier numbers in this repo

The instrumentation was temporary and is **not** in this change: a
`profile` module in `simulator.rs` with 20 `AtomicU64` slots
(`ENGINE_SIM_PROFILE=1` turns on `Instant`-based section timers; off, they cost a
`OnceLock` read), plus markers in `simulator.rs`, `chamber.rs`, `scs.rs`,
`synth.rs`, `resolve.rs` and `expansion.rs`, driven by a throwaway example that
ran the frame loop and the audio path exactly as the CLI does. Rebuilding it takes
minutes — the skeleton is in [§9](#9-how-to-re-measure).

Two rules the numbers follow, learned by getting them wrong first:

1. **A section's clock must start after the previous section's `add`.** Overlapping
   `Instant`s made `render_audio_block` once appear to take 1.58 s of a 1.58 s run.
2. **A child's total must not exceed its parent's.** The first pass of this
   investigation reported "`chamber.update` is 58% of wall" — wrong by more than
   an order of magnitude, because slot 7 was labelled `chamber.update` but
   measured `simulate_step_internal` as a whole, and the machine-level picture was
   read off the label instead of the call sites. The correct reading is §3. Every
   table here was checked with rule 2 before it was believed.

Instrumentation inflates: the same 2 s run took 1.53 s clean and 1.70 s instrumented
(11%). Shares are quoted from the instrumented run and marked as such; wall-clock
claims are always clean.

---

## 3. The frame, in full

`assets/main.mr`, 2 s of audio at 40 kHz, 109 frames, 88 207 output samples,
clean wall **1.49–1.53 s**, setup 0.60 s. Instrumented:

| section | total | calls | per call | share (of 1.70 s) |
|---|---|---|---|---|
| `start_frame` | 0.02 ms | 109 | 0.2 µs | — |
| **`simulate_step` loop, per frame** | **1 400 ms** | 109 | 12.8 ms | **82%** |
| ↳ `step_solver` | 277 ms | 40 003 | 6.9 µs | 16% |
| ↳↳ `process_constraints` | — | — | — | **89% of the solver** (see below) |
| ↳↳ `chambers.apply` (the solver closure) | — | — | 193 ns | 3% of the solver |
| ↳ engine + vehicle update | 2.7 ms | 40 003 | 67 ns | 0.2% |
| ↳ transmission update (one `Mutex` lock/step) | 1.7 ms | 40 003 | 43 ns | 0.1% |
| ↳ dyno bookkeeping | 2.7 ms | 40 003 | 67 ns | 0.2% |
| ↳ **`simulate_step_internal`** | **1 098 ms** | 40 003 | 27.4 µs | **65%** |
| ↳↳ prologue (angles, ignition, VTEC) | 2.2 ms | 40 003 | 56 ns | 0.1% |
| ↳↳ `chamber.update` × 4 + flow resets | 80 ms | 40 003 | 2.0 µs | 4.7% |
| ↳↳ **fluid loop** (8 sub-steps) | **1 009 ms** | 40 003 | 25.2 µs | **59%** |
| ↳↳↳ `chamber.flow`, 4 cylinders | 859 ms | 320 024 | 2.68 µs | 50% |
| ↳↳↳ exhaust + intake `process` | 122 ms | 320 024 | 382 ns | 7% |
| ↳ `write_to_synthesizer` | 6.9 ms | 40 003 | 173 ns | 0.4% |
| `end_frame` | 0.01 ms | 109 | 0.1 µs | — |
| **`render_audio_block` + `read_audio_output`** | **315 ms** | 109 | 2.9 ms | **18.5%** |

Inside the solver, measured on a separate 1 s run of the same script (55 frames,
20 185 steps, so that both sides are the same run): `step_solver` 144.6 ms,
`process_constraints` 128.7 ms — **89%** — and the closure that applies the
chambers' pressure and friction forces 3.9 ms (193 ns per call, 4 chambers, so
~48 ns per chamber per solver iteration). The constraint solve is the solver; the
chamber forces are not.

Inside `chamber.update` (40 003 steps × 4 chambers, ~500 ns each):

| | per call | share of the 2.0 µs loop |
|---|---|---|
| `get_volume` | 78 ns | 16% |
| `update_cycle_states` (`piston_speed`, `pressure` sample write) | 45 ns | 9% |
| `intake_flow_rate` → `sample_triangle` | 100 ns | 20% |
| `exhaust_flow_rate` → `sample_triangle` | 94 ns | 19% |

So a `sample_triangle` over a 5 636-sample lobe table costs ~100 ns: the triangle
filter's walk is short, and it is *not* a hot spot, which is the opposite of what
"per-chamber, per-step curve sampling" sounds like.

Inside `chamber.flow` (2.68 µs per fluid sub-step per cylinder): one `get_volume`
(36 ns), four `GasSystem::flow` calls, `flow_env`, two `update_velocity`, one
`change_energy`. That is ~20 small thermodynamic functions at 40–130 ns each. The
`powf`/`sqrt` in `flow_rate` are ~30 ns apiece and there are at most a handful per
step — visible, but not dominant.

---

## 4. Per engine: the same three costs, in different proportions

1 s of audio each, instrumented, percentages of the frame-loop wall time.

| engine | steps/s | fluid | solver | audio | chamber.update | IR taps | channels |
|---|---|---|---|---|---|---|---|
| `assets/main.mr` | 40 003 | **59%** | 16% | 18% | 4.7% | 5 636 | 1 |
| `radial_9` | 7 590 | **33%** | **30%** | **34%** | 3.1% | 10 000 | 2 |
| `honda_trx520` | 40 370 | **45%** | 12% | 37% | 2.6% | 10 000 | 2 |
| `ferrari_412_t2` | 5 060 | 39% | 28% | 28% | 4.1% | 5 979 | 1 |
| `harley_shovelhead` | 35 310 | **53%** | 15% | 25% | 3.7% | 5 979 | 1 |

Two regularities fall out, and they generalise the rest of this document:

* **Fluid cost** = `audio seconds × steps/s × 8 fluid sub-steps × cylinders × ~0.65 µs`.
  Checks: `main` 1 × 40 003 × 8 × 4 × 0.65 µs = 832 ms (measured 859); `radial_9`
  1 × 7 590 × 8 × 9 × 0.65 = 355 ms (measured 349); `trx520`
  1 × 40 370 × 8 × 1 × 0.65 = 210 ms (measured 192 — one cylinder). The
  per-chamber-per-sub-step constant is 0.60–0.65 µs on every engine.
* **Audio cost** ≈ `samples × taps × 0.63 ns` per chain, independent of the
  engine. `radial_9`: 44 100 × 10 000 × 0.63 ns ≈ 278 ms per chain, and the whole
  audio path measured 407 ms with its two channels interleaved — i.e. the audio
  path is essentially *only* the convolution, and the rest of the synthesizer
  (jitter, DC, derivative, noise, anti-aliasing, leveler) is noise by comparison.

The step rate is the script's `simulation_frequency`, so "steps/s" varies 8× across
the corpus (5 kHz for the 412 T2 to 40 kHz for the small singles). That is why the
solver matters more on the big multi-cylinder engines and the fluid loop matters
more on the high-frequency singles.

---

## 5. O1 — the front end is quadratic (prototyped, measured, exact)

Setup is not a rounding error: `radial_9` spends **4.15 s** before the first frame
of a run that is then asked for half a second of audio. Every gate in the suite
pays it, per engine.

Attribution inside `resolve::program` (21 units in, for every script in the corpus):

| script | units | load | lower | resolve | **expand** | **build_graph** | setup |
|---|---|---|---|---|---|---|---|
| `radial_9` | 21 | 2 ms | 2 ms | 1 ms | **3.467 s** | **0.655 s** | 4.15 s |
| `radial_5` | 21 | 2 ms | 2 ms | 1 ms | **1.262 s** | **0.209 s** | 1.49 s |
| `assets/main.mr` | 21 | 2 ms | 2 ms | 1 ms | **0.535 s** | **0.052 s** | 0.60 s |
| `trx520` | 20 | 2 ms | 2 ms | 0 ms | 0.273 s | 0.035 s | 0.32 s |

Lexing, lowering, linking and definition resolution together are ~5 ms. Everything
is `expand_all`.

Counting the work explains it. Expansion stores its memo as
`HashMap<StructureId, Vec<(Option<ContextId>, Option<StructureId>)>>` and looks an
entry up with a **linear scan plus a structural comparison of two context chains**
(`ReferenceState::expansion`, `ContextArena::is_equal`). The number of those
comparisons, counted on the real scripts:

| script | contexts created | `find_context` steps | `is_equal` calls | chain steps |
|---|---|---|---|---|
| `main.mr` | 55 601 | 61 941 | **188 252 201** | 192 274 415 |
| `radial_5` | 87 448 | 122 447 | **441 683 881** | 461 261 109 |
| `radial_9` | 129 839 | 192 585 | **959 978 561** | 1 011 164 698 |

A thousand million comparisons to expand a nine-cylinder engine, on tables with
55–130 k entries: that is the quadratic term, and it is why the per-context cost
*rises* with engine size (8 µs, 14 µs, 28 µs) instead of staying flat.

**The fix.** Give every context node a hash of its chain, computed once when the
node is pushed (`hash = mix(parent.hash, context)`, zero for a frame with no
context, terminator at the first such frame, `main` excluded because `is_equal`
ignores it), and bucket the memo by `(structure, chain hash)`:

```rust
pub fn expansion(&self, structure: StructureId, context: Option<ContextId>) -> Option<Option<StructureId>> {
    let bucket = self.expansion_buckets.get(&(structure, self.chain_hash(context)))?;
    bucket.iter()
        .find(|(candidate, _)| self.contexts.same(*candidate, context))
        .map(|(_, expansion)| *expansion)
}
```

`set_expansion` is the same shape, with `entry(...).or_default()`.

**Why it cannot change anything.** The proof is one direction: *equal chains hash
equal.* `is_equal` compares the `context` field of each frame up to and including
the first frame with no context, and never looks at `main`; the hash folds exactly
those fields, in that order, from the node up to and including that same first
empty frame. By induction on depth, any two chains that compare equal fold to the
same value. Therefore a bucket miss proves that no equal entry exists anywhere in
the structure's table, so returning `None` is what the scan would have returned —
and a bucket hit is still confirmed by `same()`, so a hash collision can only cost
time, never a wrong answer. The order question that sinks the obvious shortcut
(a *hint index* keyed by context id) does not arise: every member of a bucket is a
different chain, so the scan inside a bucket is the same first-match scan as
before, over a set that is closed under the equivalence.

**Measured** (prototype, now reverted):

| script | `expand_all` before | after | setup before | after |
|---|---|---|---|---|
| `radial_9` | 3.607 s | **0.234 s** (15.4×) | 4.150 s | **0.738 s** (5.6×) |
| `radial_5` | 1.362 s | **0.149 s** (9.1×) | 1.490 s | **0.360 s** (4.1×) |
| `ferrari_412_t2` | ~0.50 s | **0.115 s** | 0.700 s | **0.209 s** (3.3×) |
| `assets/main.mr` | 0.484 s | **0.105 s** (4.6×) | 0.602 s | **0.175 s** (3.4×) |

**Verification of exactness** (run with the prototype in place):

* `examples/ir_dump` and `examples/graph_dump` for **all 20 engines plus
  `main.mr`**, before and after: `diff -r` returns **empty** for all 42 files
  (5 999 IR lines, the full graph dump). The expansion's *output and its order* are
  unchanged, which is the only thing downstream sees.
* Context counts identical (129 839 for `radial_9`, 55 601 for `main.mr`) — the
  walk creates exactly the same tree.
* `cargo test --release --all-targets`: 184 tests, 0 failures.

**Two dead ends worth recording**, both measured:

* `if left == right { return true }` at the top of `is_equal` — sound, but
  worthless here (3.48 s → 3.61 s, noise): equal contexts in this walk are almost
  always *freshly created* ids, which is exactly what the comment on the memo
  field warns about.
* A hint index keyed by context id, with `same()` still arbitrating — **wrong**,
  and the dumps said so: 129 839 → 331 965 contexts and `expand_all` 3.6 s →
  30 s. The hinted entry was a member of the same equivalence class but not the
  *first* member, so a different expansion value came back and the walk diverged.
  This is why O1 buckets by value and re-scans inside the bucket rather than
  pointing at a remembered index.

---

## 6. O2 / O4 — the convolution is a serial chain, and chains are cheap to multiply

`ConvolutionFilter::f` is `result += h[i] * shift[...]` — one dependent float add
per tap. On a 5 636-tap response that is 5 636 × ~4 cycles of latency that nothing
else can fill, which is why `f_pair` (two channels interleaved) already exists and
documents the trick.

What is missing is the case `main.mr` actually uses: **one** input channel. There
`render_audio` calls `f` on a single filter, so the audio path is one serial chain
at 1.60 G MAC/s. Two *consecutive output samples* of the same filter are two
independent chains, and the second one's sum can be formed from the first one's
register:

```
out(s)   = Σ_{i=0..n-1} h[i]   · R[(o+i) mod n]
out(s+1) = h[0]·y + Σ_{i=0..n-1} h[i+1] · R[(o+i) mod n]      (i to n-2 for the sum)
```

where `R[(o+k) mod n]` holds `x[s-n+1+k]` and `y = x[s+1]`. Both sums add *the same
operands in the same order* as `f` would add them for that sample; `out(s+1)`'s
leading `h[0]·y` is its first term, exactly as in `f`. Nothing is reassociated,
nothing is widened, no product is reused across samples.

| form | 5 636 taps | 10 000 taps | 5 979 taps | vs one chain |
|---|---|---|---|---|
| `f`, one chain | 1.58 G MAC/s | 1.58 | 1.49 | — |
| `f_pair`, two channels (`main` never reaches this) | 2.48–2.55 | — | — | 1.56× |
| **`f_two`, one filter, two samples** | **3.15** | 3.12 | 3.01 | **1.99×** |
| four samples per pass (probe) | 3.29 | 3.29 | 3.29 | 2.08× |
| `f_quad`, two channels × two samples (one run) | 3.13 | — | — | 1.96× |

The probe is the important line: **four chains buy 4% over two** (2.08× against
1.99×), because two independent adds already saturate the machine's add throughput
at ~3.3 G MAC/s. Sample-parallelism has no headroom past two, so O2 stops there.
The same measurement is why O4 above is demoted: `f_pair` reaching only 2.5 G MAC/s
while a *one-register* two-chain form reaches 3.15 points at the two-response
stream pattern, not at the number of chains.

Bit-identity: `f_two` vs two `f` calls, **bit patterns compared** (not floats) for
every wrap offset, `n = 2..40`, 1 000 consecutive pairs each — all equal, including
`-0.0`. `f_quad` the same, `n = 2..8` × 400 pairs.

Effect on a frame: `main.mr`'s audio path is 315 ms of which the convolution is
~310 ms (see §4); halving it saves **~155 ms of 1.53 s ≈ 10%**. For the stereo
engines the gain is the smaller `f_pair` → `f_quad` step (~25% of the convolution,
≈ 5–8% of a frame).

**What it costs in code.** `render_audio` currently renders one sample per call;
`f_two` needs the *next* sample's two channel inputs, which means a two-sample entry
point in `Synthesizer` and a `render_block` loop that steps two at a time with a
one-sample tail. The filter state update becomes `shift_register[o-1] = y;
shift_offset = (o + n - 2) % n`, i.e. exactly what two `f` calls do.

**Rejected here and permanently**: partitioned/FFT convolution, wider accumulators,
four partial sums per output sample, `f32x4` lanes. Every one of them reassociates
the sum, and the audio is compared byte for byte (`WAVE` gate, `wave_dump`, the
recorded impulse-response fixtures). `f_pair`'s own doc comment already says this;
it is repeated here because "vectorise the convolution" is the first thing anyone
proposes.

---

## 7. O3 — one volume per timestep, not eight

`chamber.flow` recomputes `self.get_volume(bodies)` on every one of the eight fluid
sub-steps, and the bodies cannot move between them (the solver ran once, before
`simulate_step_internal`). Measured: 36 ns per call, 1 280 096 calls for 2 s of
`main.mr` = **46 ms** (3.0% of wall, 4.6% of the fluid loop).

The hoist is exact by inspection: same pure function, same argument, and the
argument's inputs are not written between the eight calls. The change is one extra
`f64` parameter on `CombustionChamber::flow` (or a cached field set in
`simulate_step_internal`), and the gate is the whole simulation corpus — a
mismatch would show up in the first `RUNNING` trace.

Why only 3%: `get_volume` is 78 ns, not 800. This is the kind of win that is worth
taking because it is free, not because it changes the shape of the problem. The
engines with the most to gain are the high-frequency four-cylinders
(`trx520`: 8 × 4 calls per step at 40 kHz ≈ 14% of its fluid loop).

---

## 8. What is left, and why it is left

| area | measured | why no proposal |
|---|---|---|
| `sample_triangle` (lobe/flow lookup) | 100 ns × 2 per chamber per step = 2.4% of a frame | the loop is already tight and bounded by the filter radius; caching across steps changes nothing (inputs differ every step) and caching *within* a step is already single-call |
| `chamber.update`'s remaining half | ~190 ns per chamber beyond the 317 ns above | below the noise floor of any safe win |
| dyno bookkeeping, `transmission` `Mutex`, engine/vehicle update | 0.1–0.2% each | a `Mutex` lock per step costs 43 ns; making it lock-free would be a design change for 0.1% |
| `write_to_synthesizer` | 173 ns/step, 0.4% | already avoids the per-cylinder `head` clone that used to dominate the radial (comment in the code records it) |
| `start_frame` / `end_frame` | 0.02 ms per frame, ~0% | nothing there |
| `assert!(force.is_finite())` in `apply` | inside the 193 ns apply | keep it: it is a compare, and it is the only cheap guard that turns a NaN into a panic instead of into silent garbage |
| `build_graph` | 0.45 s for `radial_9` after O1 (was 0.66 s) | **O5**: now the largest single item in setup, and unattributed. A `graph` build should be linear; profile it before proposing anything |
| `GasSystem::flow`'s `powf`/`sqrt` | ~30 ns each, a handful per sub-step | `powf(x, k)` → repeated multiplication is **not** bit-identical (libm's `powf` is not correctly rounded, and `x*x*x` differs in ulps). Off limits — this is the single most tempting wrong answer in the gas path |
| fluid sub-step count (8) | 59% of a frame scales directly | this is a *physics* parameter of the reference; halving it would halve the cost and change every number. Off limits |

---

## 9. How to re-measure

The instrumentation was removed so that no profiling scaffolding sits in the hot
path of the shipped simulator. To rebuild it:

1. Add to `rust/crates/engine-sim-core/src/simulator.rs`, at module scope:

   ```rust
   pub mod profile {
       use std::sync::atomic::{AtomicU64, Ordering};
       pub const SECTIONS: usize = 20;
       pub static NANOS: [AtomicU64; SECTIONS] = [const { AtomicU64::new(0) }; SECTIONS];
       pub static COUNTS: [AtomicU64; SECTIONS] = [const { AtomicU64::new(0) }; SECTIONS];
       pub static NAMES: [&str; SECTIONS] = [ /* one label per slot */ ];
       pub fn on() -> bool {
           static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
           *FLAG.get_or_init(|| std::env::var("ENGINE_SIM_PROFILE").is_ok())
       }
       pub fn add(section: usize, nanos: u64) {
           NANOS[section].fetch_add(nanos, Ordering::Relaxed);
           COUNTS[section].fetch_add(1, Ordering::Relaxed);
       }
       pub fn report() -> String { /* label, ms, calls, ns/call, share */ }
   }
   ```

2. Bracket the sections listed in §3 with
   `if prof { profile::add(k, clock.elapsed().as_nanos() as u64); clock = Instant::now(); }`,
   taking a **fresh `Instant` after every `add`**.
3. For the front end, time `load_graph` / `lower` / `link_imports` /
   `resolve_all_definitions` / `check_circular_definitions_all` / `expand_all` /
   `build_graph` separately inside `resolve::program`.
4. Drive it from a throwaway example that mirrors the CLI: `resolve::program(script,
   &["../../es/", "../es/", "es/"])`, `build::simulator`, then a `1/60`-second
   frame loop with `start_frame`/`simulate_step`/`end_frame`/`render_audio_block`/
   `read_audio_output`. Frame counts are history; 6 000 frames at `1/60` is the
   only honest way to run 100 s.
5. For the convolution, `examples/conv_bench.rs` already isolates `f` and `f_pair`
   (taps, samples from argv); add the `f_two` form to it when O2 lands.

---

## 10. What is verifiable here, and what is not

Everything in this document that claims *exactness* is claimed against a gate that
exists in this repository:

| claim | gate |
|---|---|
| O1 changes no resolved structure | `RESOLVED` (40 scripts vs the oracle's rows), `IR`, `GRAPH` dumps |
| O1 changes no expansion order | `ir_dump`/`graph_dump` `diff -r` over 21 scripts (done, empty), plus `cargo test` 184/184 |
| O2/O4 change no audio sample | `WAVE` gate (recorded fixtures, byte compare), `wave_dump`, `dsp_trace`, the `dsp` unit tests that already assert `f_pair` equals two `f`s |
| O3 changes no simulation state | `DRIVE`, `DRIVEN`, `RUNNING`, `ENGINES` traces against the oracle |

What is **not** verifiable in this environment, and is therefore not claimed:
real-time pacing against a sound card (there is no device here; the CLI's
`--timing` is the closest thing), GUI frame pacing (the shell is not ported), and
anything about `delta-studio` (submodule not checked out). O1–O4 touch none of
those.

---

## 11. Recommendation

1. **Land O1 now, on its own PR.** It is a front-end indexing change with no
   arithmetic in it, its exactness evidence is already collected and cheap to
   re-run (`ir_dump`/`graph_dump` `diff -r`, `cargo test`), and it makes every
   subsequent gate and every developer iteration 3–5× cheaper to start. It also
   *reduces the risk* of the rest of the plan: the drivetrain and CI phases both
   run the front end dozens of times per invocation.
2. **Land O2 with it or immediately after**, wired into `Synthesizer` as a
   two-sample render path with a tail, gated by `WAVE` plus a new unit test that
   asserts `f_two` equals two `f`s on the real 5 636-tap response (the prototype's
   bit comparison of `n = 2..40` becomes a test, not a throwaway bench).
3. **Take O3 in the same PR as the next change to `simulator.rs`** — it is four
   lines and the corpus verifies it.
4. **Do not take O5, or anything in §8, without a profile of its own.** In
   particular: `build_graph` is now the second-largest setup cost and nobody has
   looked inside it.
5. **Correct the plan's numbers** in the PR that lands O1: `docs/implementation-plan.md`
   quotes the suite's 5 m 50 s and per-engine setup as fixed constants. After O1,
   the setup component of `RUNNING` (20 engines) drops from ~16 s to ~4 s, and the
   whole suite by roughly 10–15 s — worth re-measuring rather than estimating,
   because the gates' own setup dominates the *small* gates and the simulation
   dominates the large ones.

The one-line version: **the port is not slow because of a mistake; it is slow
because it is simulating an engine, and the two places where it is slow for a
reason that is not physics are the expansion memo and a mono convolution that only
ever runs one chain.**
