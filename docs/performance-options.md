# O1–O3, contested

[`performance-investigation.md`](performance-investigation.md) ranked six
optimisations and recommended the top three. That is a list, not a decision. This
document takes each of O1–O3 in turn, writes down **every** way to get that win
that I could defend — including the ones that are wrong, because two of them were
wrong in ways that only measurement caught — contests them on pro / con / risk /
outcome, and then says what I would actually do, which is usually **not** any
single option in the table.

Standing rule, unchanged: an optimisation that changes one output bit is a
fidelity bug, not a speed-up. So each option carries the gate that would prove it
innocent, and *measured* means a number from this machine, not an estimate.

Everything here was run; the code was reverted afterwards. Two new measurements
this turn change a conclusion from last turn (O2 / O4 — see §2.6).

---

## 0. Scorecard

| item | worth (measured) | verdict | evidence |
|---|---|---|---|
| **O1** expansion memo | `expand_all` 3.61 s → 0.23 s on `radial_9` (15×), whole front end 4 115 ms → 747 ms | **landed** (`55b0983`), option E, with the hash invariant asserted; **F** remains a sequel, not an alternative | dumps of 21 scripts identical; 184 tests; all 13 oracle gates |
| **O2** mono convolution | 1.99× the convolution ⇒ **14.3%** off a `main.mr` frame | **landed** (`0ea3870`), option B only; C (four chains) rejected on measurement | bit-verified `n = 2..40` and the real 5 636-tap response; k=2 vs k=4 timed |
| **O3** volume per sub-step | 3% of a `main.mr` frame, ~3% of `trx520`'s | **landed** (`80e79e7`), option D+A (one evaluation per timestep, threaded as a parameter) | 36 ns × 8 sub-steps × cylinders, counted |

---

## 1. O1 — the quadratic expansion memo

**What it is worth.** `resolve::program` spends 0.32 s (`trx520`) to 4.15 s
(`radial_9`) before a run starts, and ≥ 84% of it is `expand_all`, which performs
**960 million** structural context comparisons to expand a nine-cylinder engine
(§5 of the investigation). Every gate in the suite pays it per engine.

The memo is `HashMap<StructureId, Vec<(Option<ContextId>, Option<StructureId>)>>`
and a lookup is a linear scan whose every step compares two context *chains* by
value (`ContextArena::is_equal`), because the reference compares context trees by
value rather than by pointer. That value-comparison is load-bearing: it is why an
expansion found under one freshly-created context is reused for an
equal-but-distinct one.

### The options

| # | option | pro | con | risk | outcome |
|---|---|---|---|---|---|
| **A** | **Do nothing in code; cache at the process level** — resolve each script once per suite invocation, or persist the built graph to disk | zero semantic risk; helps CI immediately | helps *only* CI; every single run, every library user and every developer still pays; adds a cache-invalidation surface (script mtimes, `es/` contents, rules) that did not exist | a stale cache silently produces a different engine — the worst possible failure mode | **rejected as the primary fix**; the plan's `--only` gate filter already captures the cheap part of it |
| **B** | **O(1) fast-reject inside `is_equal`** (compare ids first, or chain hashes) | 5 lines, no structural change, obviously sound | the cost is not the *walk*, it is the *number of comparisons*: the average walk is 1.05 frames, so every comparison is already ~1 step | none — but it buys nothing | **rejected, measured**: 3.48 s → 3.61 s (noise) |
| **C** | **Memoise `same(a, b)` by id pair** | trivial; sound | ~10⁹ distinct pairs; unbounded memory for a cache whose entries are almost never revisited | memory blow-up on a bigger script | **rejected without measuring** (the arity argument is decisive); recorded so nobody tries it |
| **D** | **Hint index keyed by exact context id** — `HashMap<(StructureId, ContextId), usize>`, falling back to the scan | small; sounds right; keeps every semantic | — | **wrong**: within an equivalence class the *first* inserted member wins, and a hint points at whichever member was remembered. Diverges: 129 839 → **331 965** contexts, 3.6 s → **30 s** | **rejected, measured, and kept in the record**: this is the trap that the memo's own comment warns about ("keying by context id loses every expansion the moment the walk creates an equal-but-distinct child") |
| **E** | **Bucket the memo by a structural chain hash**, verify with `same()` *inside* the bucket | 15× measured; a bucket miss is a *proof of absence* (equal chains hash equal), so no scan is needed; first-match order is preserved because every member of a bucket is a different chain; the hash is derived from exactly the fields `is_equal` compares | the hash has to reproduce two quirks of `is_equal` — it ignores `main`, and it stops at the first frame with no context; get either wrong and behaviour changes *quietly* | the correctness argument is "equal ⇒ equal hash" and it lives in a comment. If someone later makes `is_equal` compare `main` (or stop differently), the hash becomes coarser and the memo starts merging classes that the reference keeps apart | **adopt**, with the invariant turned into an assertion (below) |
| **F** | **Intern contexts at creation** so equal chains share one id; then the memo is keyed by id and `is_equal` collapses to `a == b` everywhere | the clean end state: removes 10⁹ comparisons *and* the O(depth) walk at 13 other call sites; memory drops (130 k contexts → far fewer) | interning must intern *exactly* the relation `is_equal` defines, which ignores `main` and stops at the first empty frame — a canonical key that is coarser than `(context, main, parent)`; ids change | the same class-merging failure as D, but now in the *identity* of contexts, so the blast radius is the whole front end rather than one table | **not first**; it is the sequel to E, and E's assertion is exactly the oracle F needs |
| **G** | **Drop the memo / expand lazily** | simplest code | the memo *is* the semantics: it is the cycle guard and the reason a value-equal context reuses an expansion | changes what "expanded" means | **rejected** |

### Where the contest lands

E, and then F, with three additions that are the difference between "a fast hack
that happens to pass" and "a fast design that cannot silently rot":

1. **Turn the invariant into a failing assertion, not a comment.** `ContextArena`
   gets `debug_assert!` on the two directions where the hash is used: whenever a
   bucket hit or miss is decided, assert `is_equal(a, b) == (hash(a) == hash(b))`.
   In debug builds this costs one chain walk per bucket probe — the thing we are
   optimising — and that is fine: it is exactly the property that must never
   change. In release it compiles out. This is also the oracle for F.
2. **Make the dump-diff a gate.** Last turn's evidence was `ir_dump` + `graph_dump`
   for all 21 scripts, before and after, `diff -r` empty — collected by hand. It
   costs ~20 s and it is the only check that sees expansion *order* as well as
   values. It belongs in the suite next to `RESOLVED`, not in a commit message.
3. **Document the two quirks in `is_equal` as load-bearing**: it compares the
   context of each frame up to and including the first frame with no context, and
   it never compares `main`. Both are now part of the hash's contract.

Then, separately, F — with (1) as its oracle and (2) as its acceptance test.
F is where the remaining wins are (the other 13 `same()` call sites), and it is
much easier to argue once the relation has an executable definition.

**What this does not fix.** `expand_all` 0.23 s is not zero: it is now a linear
walk with an allocation per bucket key, and `build_graph` (0.45 s on `radial_9`)
is untouched and unattributed. That is O5, and it should be profiled before
anyone proposes anything.

---

## 2. O2 — the convolution

**What it is worth.** The audio path is 18% of a `main.mr` frame, 25% of
`shovelhead`, 34% of `radial_9`, 37% of `trx520` — and it is almost entirely the
impulse-response convolution. `ConvolutionFilter::f` is one dependent float add
per tap, so a 5 636-tap response costs 5 636 add latencies per sample. `f_pair`
already interleaves the two *channels*, which is why stereo assets are not twice
as slow — but two of the corpus's assets are **mono** (`main.mr` and
`honda_trx520`; `radial_9`, `ferrari_412_t2` and `harley_shovelhead` are the
stereo ones, as `synth_channels` in the trace shows), and those ran one chain.

New measurements this turn (same machine, 88 200 samples, impulse response from
`conv_bench`'s generator; each form bit-compared against `f` over `n = 5..39` at
every wrap phase, plus `n = 2..40` for the two-sample form):

| form | 5 636 taps | 10 000 taps | 5 979 taps |
|---|---|---|---|
| `f` (one chain, what mono does now) | 1.58 G MAC/s | 1.58 | 1.49 |
| two samples per pass (prototype `f_two`) | **3.15** (1.99×) | 3.12 (1.97×) | 3.01 (2.02×) |
| four samples per pass (new prototype) | 3.29 (**2.08×**) | 3.29 (2.08×) | 3.29 (2.22×) |
| real `f_pair` (two channels), `conv_bench` | 2.48–2.55 | — | — |

### The options

| # | option | pro | con | risk | outcome |
|---|---|---|---|---|---|
| **A** | **Status quo** | zero risk | mono leaves 2× of the audio path on the floor; every mono asset pays it | none | baseline |
| **B** | **Two consecutive samples per pass**, shifted taps (`h[i+k]` against one register) | **1.99×** measured, bit-identical; a self-contained function; no state change; the second sample's terms are the first's operands with the taps shifted, so nothing is reassociated | index-heavy: region split at the wrap × per-chain tail; needs a two-sample entry point and a tail case in `render_block`; the algebra is right up to the boundary and wrong *at* it | a boundary bug shows up only near wrap points — which occur every `n` samples, so `WAVE` over 100 s of audio is a strong net | **adopt** |
| **C** | **Four samples per pass** | sounds like more of a good thing | **2.08× vs 1.99×**: 4% of the audio path, under 1% of a frame; four seeds, three nested tails, three extra boundary cases | more boundary cases for ~nothing | **rejected, measured**: i.e. reaching 3.29 G MAC/s against a ceiling of ~3.3 |
| **D** | **Mirrored shift register** — keep the ring in three copies so each output's window is one contiguous slice, no split loops | would remove the region split entirely; one tight loop; the natural way to try K samples at once | **does not work for K > 1**: the ring holds `n` samples, so writing two pending samples overwrites the first one's window — the shifted-tap algebra in B exists precisely because of that. For K > 1 it needs a copy per chain (90 KB at four chains, K× the register streams) | a silent wrong answer at the wrap | **rejected by construction and by a prototype that failed its own bit test**; recorded so it is not re-derived |
| **E** | **SIMD across samples** (`f32x4`, four consecutive outputs in four lanes) | lanes are independent sums in their own order ⇒ exact by construction; fewer instructions | `std::simd` needs nightly (toolchain is pinned at 1.88); `core::arch` needs per-arch paths and runtime detection across the CI matrix; and it cannot beat C's ceiling because the bottleneck is add throughput, not instruction count | portability churn in the one crate that must stay bit-exact everywhere | **rejected for now** — revisit only if a profile ever shows the convolution below ~3.3 G MAC/s |
| **F** | **Fix the stereo layout** — the two channels' responses interleaved into one array (or two rings in one allocation) | the real `f_pair` measures **2.48–2.55 G MAC/s while the demonstrated ceiling on this loop shape is 3.3** — i.e. ~25% is being lost to four memory streams, not to latency; that is 8–9% of a stereo engine's frame | changes the filter's stored layout and the IR-recording path that fills it | layout-only, so exactness is easy to argue — but it is a *hypothesis* about a 25% gap, not a result | **parked as O2b, data first**: one measurement (interleaved layout, same bits) before it becomes a proposal |
| **G** | **FFT / partitioned convolution / wider accumulators / tap-parallel SIMD** | 5–10× in principle | every one reassociates the sum | changes the audio bytes | **rejected, permanently** |

### Where the contest lands

B, and only B, plus three things that make it cheap to own:

1. **A permanent test, not a bench.** The prototype's check — `f_two` equals two
   `f` calls, bit for bit, for every wrap phase — becomes a unit test on the real
   5 636-tap response over at least three wrap periods, with `to_bits()`
   comparisons (not float equality, so `-0.0` is caught). That test is the entire
   safety argument; without it the algebra is a comment.
2. **Keep the general path.** The two-sample entry point renders a pair while it
   can and calls the existing `f` for the odd tail — so the one-sample function
   never dies and the stereo path keeps working unchanged.
3. **Demote the stereo idea from a claim to a task.** Last turn's doc said four
   chains give 1.25× on stereo from a single run against a slower baseline; the
   new data says two chains already saturate the mono shape, so the stereo gap is
   a *layout* effect (F) and needs its own measurement. The investigation doc is
   corrected to say so in this same change.

---

## 3. O3 — the volume recomputed eight times

**What it is worth.** `simulate_step_internal` runs the fluid loop eight times per
timestep (the reference's `fluid_simulation_steps`), and every
`CombustionChamber::flow` call starts with `get_volume(bodies)` — 36 ns, 8 ×
cylinders times per timestep, **1 280 096 calls per 2 s of `main.mr` = 46 ms
(3.0%)**. The bodies cannot move between those eight calls: the solver ran once,
before the loop. `chamber.update` computes the same value again, once per chamber
per timestep (78 ns × 4 = 0.8% more). Call sites are exactly one each
(`simulator.rs:622` for `update`, `:643` for `flow`), so this is a local change.

### The options

| # | option | pro | con | risk | outcome |
|---|---|---|---|---|---|
| **A** | **Pass the volume into `Chamber::flow`** as a parameter | explicit; no state, no invariant to remember; one call site | signature grows by one `f64` | none | **adopt** |
| **B** | **Cache it in the chamber** (`flow_volume` set by `update()`) | no signature change; `update` can reuse it too | creates an ordering invariant ("flow must follow update") that nothing enforces | a future caller gets a stale volume and produces a *wrong engine* rather than a crash — the failure mode we care most about | **rejected as the mechanism**; A gets the same win with no invariant |
| **C** | **Memoise inside `get_volume`** (last theta → last volume) | covers all five callers at once, 3 lines | a cache inside a function used on error-adjacent paths; must be invalidated when geometry changes (setup, `place_cylinder`) | unbounded in time: stale after geometry edits, silently | **rejected** — the invariant's lifetime is unbounded, unlike A's |
| **D** | **One evaluation per timestep in `simulate_step_internal`**, feeding both `update` and the eight `flow` calls | saves the four `update` calls *and* the 32 `flow` calls; single evaluation site; the invariant is "bodies do not move inside `simulate_step_internal`" — provable by looking at one function | needs a small scratch buffer (a reusable field, not a per-step `Vec`) | none beyond A | **adopt, with A** — 25% more of the win for the same invariant |
| **E** | **Do nothing** | 3% is not much; every line of this is complexity for a number no user sees | leaves 3% (14% of `trx520`'s fluid loop) on the table for ~15 lines | none | **loses to A+D**: the corpus verifies it and the diff is small |

### Where the contest lands

D + A: compute each chamber's volume once at the top of `simulate_step_internal`,
where the reference first needs it, and pass it to `update` and to `flow`. Say it
in the type: `flow(&mut self, dt: f64, volume: f64, ...)` and
`update(&mut self, bodies: &[RigidBody], volume: f64, ...)` — a parameter cannot go
stale, and the compiler makes every future caller think about where the number
comes from.

That is the entire change: one loop in `simulator.rs`, two signatures, no new
state. `DRIVE` / `DRIVEN` / `RUNNING` verify it, because a stale or mis-timed
volume moves the pressure trace on the first frame.

---

## 4. The risk that any of this changes an output bit

The short answer, per item: **O1 and O3 carry no floating-point arithmetic at all**
— one is a compile-time table index, the other is an `f64` reused instead of
recomputed — so they cannot move a number by rounding. O2 does arithmetic, and it
is the only one where the numbers themselves are at stake; its algebra is proven
per-output-sample bit-identical and tested as such. The interesting part is not
the size of the risk, it is *what would catch it*, and there the answer is uneven:
the trace gates are stronger than they look for the simulation and weaker than
they look for the audio.

### 4.1 What the change can do to the numbers

| | adds/removes/reorders float ops? | what can go wrong | how it would show | residual after the gates |
|---|---|---|---|---|
| **O1** | **none** | (a) the hash is *finer* than `is_equal`, so a bucket miss wrongly returns "no expansion" → wrong or missing expansion → a different IR; (b) the memo keys on context ids instead of values → the wrong member of an equivalence class wins → a divergent walk; (c) someone later edits `is_equal` and the hash silently goes stale | a different IR, which means different geometry, different gas state, different everything: `RESOLVED`, `IR`, `GRAPH`, `VALUES`, `OBJECTS`, `ENGINE` and every trace fail at once — loud, not quiet | low. (b) is already measured and fatal-looking (331 965 contexts, 3.6 s → 30 s). (a) cannot happen if the derivation is right — the hash reads exactly the fields `is_equal` reads, in the same order, with the same stopping rule ⇒ *equal ⇒ equal hash* for every chain, corpus or not. (c) is the real one: it is a maintenance hazard, and it is why the `debug_assert` in §1 exists |
| **O2** | **none numerically** — the same products in the same order, split across two independent accumulators | (a) an algebra error at a wrap boundary → wrong samples for some phases; (b) the plumbing reorders the per-sample work — two `rng.next_unit()` draws per channel per sample, the jitter/DC/derivative/air-noise state, the leveler and antialias filters all advance once per *sample*; if a two-sample path computes both samples' inputs before convolving, the draw order changes and the noise changes | (a) and (b) both change audio bytes: the trace emits every rendered `i16` sample it reads, and every other emitted field keeps matching, so the failure localises to audio immediately | low. (a) is closed by test, not by hope: the prototype's bit comparison covers `n = 5..39` at *every* wrap phase, which is every boundary the algebra has. (b) is loud. The genuine residual is (c): **the trace renders only ~2 samples per frame** (the trace driver advances one simulation step per frame, so the input ring is nearly empty and `render_block` has almost nothing to do) — a bug that only appears when hundreds of samples render at once is outside what the corpus exercises. That is a gap in the *gates*, not in the change, and §4.4 proposes closing it |
| **O3** | **none** — the same `f64` value, computed once instead of nine times | the value is computed at a point in the step where the bodies have moved | `chamber_volume`, `crank_theta` and every pressure field diverge on the first step — the loudest possible failure | very low. `get_volume` reads only `bodies[piston_body].p_x/p_y` and the bank's geometry, and bodies do not move between `chamber.update` and the eighth `flow` call. Passing it as a parameter means a future caller has to produce it, rather than inherit a stale field |

The rejected options in §2 are exactly the ones with real numeric risk: FFT /
partitioned convolution, wider accumulators, tap-parallel SIMD, four chains with
per-chain register copies (which changes *which* register the taps multiply),
`powf` → repeated multiplication, fewer fluid sub-steps, a shorter impulse
response. Each of those trades bits for time. That is why they are rejected and
not deferred.

### 4.2 What the suite actually compares

This matters because "the gates are green" is doing different amounts of work for
different fields:

| what | granularity | where |
|---|---|---|
| resolved script, node graph, values, objects, settings, engine configuration | text, compared to the oracle's own dump | `RESOLVED`, `IR`, `GRAPH`, `VALUES`, `OBJECTS`/`SETTINGS`, `ENGINE`, `EXECUTE` — 40 scripts |
| simulation state (crank, piston, rod, chamber, gas, dyno, ignition) | `f64` **bit patterns** (`{:016x}`), per frame | `RUN` (400 frames), `DRIVEN` (20 000 frames = 20 000 steps), `RUNNING` (all 20 engines, 0.5 s each), `ENGINES` (every asset, 400 frames) |
| rendered audio | `i16` samples, as the app writes them | the same traces — but **~2 samples per frame**, because the trace driver steps the simulation once per frame and the synthesizer's input ring is therefore almost empty |
| impulse-response decoding | every sample, byte compare | `WAVE` — this is the RIFF decoder on `es/sound-library/*.wav`, **not** the rendered engine audio |
| full-rate rendered audio (the 44.1 kHz stream the app plays) | nothing | **no gate compares it today**, against the oracle or against a recording |

Two consequences:

* For O2, the oracle comparison of audio is real but thin, and the *thick* check is
  the unit-level bit test. That division of labour has to be explicit, or someone
  will assume the trace covers the convolution at full rate and stop writing the
  unit test.
* The simulation fields are compared as `f64` bits, which is why O3's failure mode
  is so loud — a stale volume cannot hide in a rounded decimal.

### 4.3 Why "prototype was bit-identical" is sound evidence here

The prototypes were checked against the **port**, not against the C++ oracle: IR and
graph dumps of all 21 scripts `diff -r`-identical, the filter compared bit-for-bit
against the existing `f`, 184 tests green. That is a *transitive* argument, and it is
the right one: the port's baseline output is pinned to the original by the 13
gates, so preserving the baseline's bytes preserves parity with the original. It
does **not** re-derive parity, and it cannot tell you whether the baseline was
already wrong — that is what the gates are for.

For the record, the argument was not left at that level: once the changes were in
the tree, the oracle was staged (~45 s) and all 13 gates were run against it — 31
`PASS` lines, exit 0, 280 s (was 350 s before O1). So O1–O3 are verified against
the original directly, not only transitively. That run is also what caught the one
wrong number in this document: the mono/stereo split above.

### 4.4 What I would add, and what would change my mind

1. **Run the 13 gates on the branch that lands each change.** Non-negotiable, and it
   turns §4.3's transitivity into a direct comparison.
2. **O1: the `debug_assert` and the dump-diff gate** (§1). The assert is the only
   thing that catches a future `is_equal` edit, and the dump-diff is the only check
   that sees expansion *order*.
3. **O2: the permanent `f_two`-vs-`f` bit test** on the real 5 636-tap response,
   plus — new, and worth doing on its own — **a golden WAV fixture**: render
   `assets/main.mr` and one stereo engine through the CLI at full rate, commit the
   bytes, and diff them in the suite. It will not prove parity with the original,
   but it makes every future audio change loud, and it closes the "hundreds of
   samples per block" blind spot that the traces leave open today.
4. **O3: nothing extra.** The parameter is the invariant; the corpus checks it.
5. **What would change my mind on O1:** a script in the corpus whose expansion
   depends on the *order* in which equal contexts were created, in a way the dumps
   do not show. I looked for one and found none — but that is the shape of thing
   that would make bucketing unsafe rather than merely slow.

---

## 5. Sequencing, and what I would change in the plan

| order | what | why this order | gate |
|---|---|---|---|
| 1 | **O1 (E)** | independent of everything; makes every later gate and every developer command 3–5× cheaper to *start*; it is the only one that removes a superlinear cost | `RESOLVED` + new dump-diff gate + `debug_assert` on the hash invariant + 184 tests |
| 2 | **O3 (D+A)** | 15 lines, lands naturally in the first PR that touches `simulator.rs`; verified by the full simulation corpus | `DRIVE`/`DRIVEN`/`RUNNING` |
| 3 | **O2 (B)** | the largest per-frame win (≈10%) and the one with the most new code; wants a quiet tree and a permanent test | `WAVE` + new `f_two`-vs-`f` bit test |
| 4 | **O5** (`build_graph`, 0.45 s) and **O2b** (stereo layout) | both are *unknowns*, not plans: each needs its own profile/measurement before anyone writes code | — |
| never | four chains (C), mirrored ring (D in O2), SIMD (E), FFT (G), `powf` → multiplication, fewer fluid sub-steps | measured no-ops, construction failures, or they change the numbers | — |

Against `implementation-plan.md`: O1 belongs **inside Phase 0**, as the last item,
because Phase 1's CI job times are dominated by how fast the front end starts and
Phase 2 runs 20 engines per gate. O3 and O2 fit Phase 2 (the drivetrain phase,
where `simulator.rs` and the trace tooling are open anyway) — but O2 touches only
`synthesizer`/`dsp`, so it does not have to wait for the drivetrain recipe. If the
phases are strictly one-PR-each, add a **Phase 0.5 "the two free wins"** and keep
it to O1 + O3.

**What has not changed.** Setup is 0.18–0.74 s after O1, but the *simulation* is
still 59% fluid loop on `main.mr` and 33–53% elsewhere, and that cost is the
reference's physics reproduced as written. Nothing in this document proposes to
touch it, and nothing should: at that point the honest options (fewer sub-steps,
shorter impulse response) all change the output, which is the one thing this port
exists not to do.

---

## 6. What landed, and what it measures now

Three commits on this branch, one per item, each verified before it went in:

| commit | change | verification |
|---|---|---|
| `55b0983` | front end: bucket the expansion memo by a chain hash | `ir_dump`/`graph_dump` for all 21 scripts byte-identical; the `debug_assert` on the hash invariant green over the script corpus; 184 tests |
| `80e79e7` | simulator: one chamber volume per timestep | `main.mr` (16 265 lines), `radial_9` (19 282) and `DRIVEN` (808 301 lines) traces byte-identical; 44.1 kHz WAV renders byte-identical |
| `0ea3870` | audio: two samples per pass on the mono path | new bit test (every wrap phase of `n = 2..40`, plus the real 5 636-tap response); the same traces and WAVs byte-identical |

End to end, best of six runs, frame loop only (setup excluded), measured by
stashing the change and rebuilding, so both sides are the same binary
configuration:

| asset | channels | before | after | |
|---|---|---|---|---|
| `main.mr`, 2 s | mono | 1.559 s | **1.336 s** | **−14.3%** (1.28× → 1.50× real time) |
| `honda_trx520`, 1 s | mono | 0.700 s | **0.498 s** | **−28.9%** (1.43× → 2.01×) |
| `ferrari_412_t2`, 1 s | stereo | 0.812 s | 0.805 s | −0.9% (the `f_pair` path is untouched) |
| `radial_9`, 1 s | stereo | 0.991 s | 1.000 s | +0.9% (noise) |

Front end, same method: `radial_9` 4 115 ms → **747 ms** (5.5×), `main.mr`
600 ms → 169 ms. Gate suite: **350 s → 280 s**.

Two notes on that table, because they are the kind of thing that gets rounded off:

* The first stereo measurement had `ferrari_412_t2` **5% slower**, which turned out
  to be `finish_sample` not being inlined into the stereo path after it was
  extracted from `render_audio`. `#[inline]` put it back at parity. A fifteen-line
  refactor of a per-sample function is visible at 44 100 Hz.
* The mono gains are larger than the profile predicted (14% against an expected
  ~9% on `main.mr`; 29% against ~20% on `trx520`). The likely reason is that the
  instrumentation's own overhead inflated the non-audio sections, so the audio
  share read low. The measured end-to-end numbers are the ones to trust; §3–§4 of
  the investigation are for attribution, not for arithmetic.

The two items left open in §5 — **O5** (`build_graph`) and **O2b** (why the
stereo path reaches 2.5 G MAC/s where a two-chain form reaches 3.15) — were
measured next; §7 is that report, and both turned out differently from what this
section assumed.

One recommendation from §4.4 is deliberately *not* taken: the dump-diff gate. The
`IR`, `GRAPH` and `VALUES` gates already compare those dumps against the **oracle**
for all 40 scripts, which is strictly stronger than diffing the port against
itself, and it would have cost ~8 MB of fixtures. The `debug_assert` covers the one
thing the oracle cannot: a future edit to `is_equal` that the hash no longer
mirrors.

---

## 7. The two open questions, measured

*Measured 2026-11-23 on a fresh sandbox: Intel Xeon at 2.60 GHz (two cores),
`rustc 1.100.0`. **The absolute numbers in §6 came from a different machine and a
different compiler and are not comparable with these** — this machine renders
`main.mr` in 1.81–1.90 s where §6's table says 1.336 s. Everything below is either
a ratio measured on one machine in one session, or a count, which does not care.
Ratios are the part that travels; treat the absolute times as local.*

Three measurements were added to make this reproducible: `front_end_bench`
(`engine-sim-script`), `pair_bench` and `pair_two_proto` (`engine-sim-core`).
The counters in §7.1 were temporary instrumentation and are not in the tree.

### 7.1 O5: `build_graph` is one linear scan, and it is most of the phase

Phase timings, best of four runs:

| phase | `main.mr` | `radial_9` |
|---|---|---|
| parse + load | 0.003 s | 0.003 s |
| lower | 0.002 s | 0.002 s |
| resolve | 0.001 s | 0.002 s |
| `expand_all` | 0.036 s | 0.095 s |
| `build_graph` | 0.014 s | **0.118 s** |
| total | 0.057 s | 0.222 s |

`expand_all` is what O1 fixed; `build_graph` is now the larger half on a big
engine. Counters, on `radial_9`:

| | count |
|---|---|
| `generate_node` calls | 116 493 |
| — of those, uncached (real work) | 107 807 |
| `cached_instance` calls | 140 124 |
| **nodes scanned inside them** | **101 227 701** |
| scans that got as far as comparing contexts | 484 070 |
| `container_for` calls / containers scanned | 5 178 / 1 604 615 |
| `find_node_in_tree` calls / entries scanned | 2 085 359 / 4 146 233 |
| `kind()` clones | 107 807 (5.0 ms total, 46 ns each) |

`GraphNode` is **384 bytes**, so one full scan of this graph walks **625 KB** —
it does not fit in L1 and barely fits in L2. 101 million scans at ~1.2 ns each is
0.12 s: `build_graph` **is** `cached_instance`, and 99.5% of that work is thrown
away by the `ir_structure` test alone (only 484 070 of 101 227 701 scans reach a
context comparison).

**Prototype.** Bucket the nodes by `ir_structure` (`RefCell<HashMap>` on the
`Graph`, appended lazily as the node list grows) and scan only the matching
bucket, still confirming with `contexts.same`:

    build_graph, radial_9:   0.118 s  ->  0.034-0.043 s   (3x)
    front end total:         0.222 s  ->  0.140 s         (-37%)

and the graphs are **byte-identical for all 23 scripts** in the repository
(`graph_dump` diffed against the unmodified tree).

Why it is exact, in the same terms as O1: the bucket is appended in node order, so
`find` returns the first node in graph order that `cached_instance` would have
returned, and the context comparison still decides. A bucket only narrows the
scan; it cannot change which node wins. The one invariant is that a node's
`ir_structure` is final by the time it is indexed — it is assigned once, at the
three creation sites in `generate_node_ires_node`/`_uncached`, before the node is
returned to any caller, and the lazy append happens at the *start* of a later
`cached_instance` call. That is worth a `debug_assert` (every node with a
structure is in its bucket, checked when the build ends), not a comment.

What is left after it (0.034 s) is 107 807 real node generations including 5 ms of
`kind()` clones, 2.09 M `find_node_in_tree` calls and 1.6 M container scans —
each of those is ≤10 ms, so **this is the last big item in the front end**.

### 7.2 O2b: this machine wants four chains, so the stereo path has 1.7x left

The convolution at 5 636 taps (best of three, 2.60 GHz). "cycles/FMA" divides the
measured time by the cycles in it, so one chain's 3.1 is the add's latency:

| kernel | chains | G mac/s | cycles/FMA |
|---|---|---|---|
| `f` | 1 | 0.83 | 3.1 |
| `f_two` (shipped mono path) | 2 | 1.65 | 1.55 |
| `f_pair` (shipped stereo path) | 2 | 1.65 | 1.55 |
| shape: 3 samples x 1 filter | 3 | 2.48 | 1.03 |
| shape: 4 samples x 1 filter | 4 | **3.30** | 0.79 |
| shape: 2 samples x 2 channels | 4 | **3.31** | 0.79 |

**The §2 conclusion that "two chains already saturate the machine" was
machine-specific and is false here.** Four independent chains run 4x a single one,
at 0.79 cycles a multiply-add — close to the load-port limit of the loop body (one
shift-register load plus four response loads per tap). The shape bench is a
predictor rather than a claim: it reproduces `f_two` and `f_pair` to within a
percent, and it says mono has another 2x and stereo another 2x before
saturation.

**Prototype.** `pair_two` — two consecutive samples of *each* channel in one pass,
four chains, each output keeping `f`'s order (leading `h[0]*y` first, taps
ascending), with the wrap region split per channel:

| | 5 636 taps | 10 000 taps |
|---|---|---|
| `f_pair` (shipped) | 1.64 G | 1.49 G |
| `pair_two` (prototype) | 2.72 G | 2.57 G |
| speed-up | **1.66x** | **1.72x** |

Bit-checked against four `f` calls at every wrap phase of `n = 2..39` and at four
phases of the real 5 636-tap response, comparing `to_bits()` — 2 593 groups, all
identical. Three things it also measured:

* **Slicing the registers to remove bounds checks made it slower** (2.63 vs
  2.72 G): the indexing was already free, so the real kernel should stay written
  the way `f_two` is.
* **Interleaving the two responses into one array changed nothing** (1.65 vs
  1.66 G at 5 636 taps): at 22 KB a response the loop is not traffic-bound. At
  10 000 taps it partly is — `f_pair` drops to 1.49 G while the one-response
  `f_two` holds 1.65 — which is why the prototype's margin is *larger* there.
* A four-chain kernel reaches 2.7 G of the shape bench's 3.3 G. The 20% is the
  wrap split and the per-pass tail: **1.7x, not 2x, is the realistic ceiling.**

**What it would be worth** (projection, from the audio share implied by the
measured `G mac/s` and the frame times on this machine — treat as an estimate):

| asset | frame | audio | with a 4-chain path |
|---|---|---|---|
| `radial_9` (stereo, 10 000) | 1.46 s | ~0.59 s | ~1.22 s (−16%) |
| `honda_trx520` (mono, 10 000) | — | — | mono, so O2 applies, not this |
| `ferrari_412_t2` / `shovelhead` (stereo, 5 979) | 1.18 / 1.10 s | ~0.35 s | ~1.04 / 0.96 s (−12%) |
| `main.mr` (mono, 5 636) | 0.93 s | ~0.30 s | ~0.78 s (−16%, at four samples a pass) |

### 7.3 What this changes about the plan

Two proposals, both in the shape of the ones already landed, both bit-exact for
the same reason (nothing is reassociated; the work is only *reordered in time*):

* **O5a — bucket the graph's nodes by `ir_structure`.** 3x on `build_graph`,
  −37% on the front end, 23 scripts verified identical. Risk: the `ir_structure`
  invariant, which a `debug_assert` covers; the `GRAPH` and `VALUES` gates cover
  the rest.
* **O2c — two samples per channel per pass.** 1.66x on the stereo convolution,
  −12% to −16% of a stereo frame; and for mono, four samples a pass is worth
  another 2x over `f_two` (measured as a loop *shape* only — a real four-sample
  kernel has three tails to handle, which is the work O2c would do). Risk: the
  same boundary algebra as O2, covered by the same kind of bit test, which now
  exists as a template. `f_pair`'s mismatched-length fallback has to be kept for
  stereo.

Neither is taken yet: this section is the measurement, and both need the same
treatment O1–O3 got (test, dumps, the 13 gates) before they land. O2c is the
bigger win and the smaller change.
