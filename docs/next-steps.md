# What is left, contested

Three pieces of work remain — enforcing the gates in CI, the application shell,
and drivetrain breadth. Each is presented as a set of options, each option
contested on its own terms, then a way forward proposed from what the contest
shows. "Outcome" is what you would *get*, not what you would hope for: an option
whose outcome is unverifiable is marked as such however attractive it looks.

Everything below is measured on this sandbox (2 vCPU, `rustc` 1.88.0, `g++ -O2`),
because the whole project's habit is to price things in gates and seconds rather
than in adjectives.

## 0. Where things stand

| fact | number |
|---|---|
| tests | 184 (`cargo test --release --all-targets`), ~10 s warm, ~2.5 min from a cold target dir |
| parity gates | 13, full suite **5 m 50 s** (`RUNNING` alone is ~2.5 min, `DRIVE` ~30 s, `DRIVEN` ~30 s, `ENGINES` ~40 s) |
| C++ oracle, from scratch | 10 s to clone `piranha`, **45 s** to stage both reference libraries, 5 s to link |
| one drivetrain-sized run | 930 ms on the port for 20 000 frames / 764 159 checkpoints; the oracle is about the same again, plus the diff |
| engine wrappers | 20, all with a `vehicle(...)` and a `transmission(...)` of their own |
| real time | 1.30 x average, slowest asset 1.02 x, setup 0.4–0.7 s (4.0 s for the nine-cylinder radial) — **on the machine that recorded it**. A 2.60 GHz Xeon, same code: 1.06 x average, slowest 0.86 x (`ferrari_f136_v8`), setup 0.11–0.41 s after O1/O5a. The original is 0.75 x average there, so the port still wins every asset, but the absolute figure is a property of the box |
| shell upstream | `engine_sim_application.cpp` 1 136 lines + `ui_*` 842 + `application_settings.h`; assessment §9 prices the Windows/Direct3D glue at ~35 k LOC, not portable |
| CI today | `.github/workflows/cmake.yml`: Windows, CMake, **C++ only**. Nothing builds `rust/`; no gate runs anywhere but a person's shell |

---

## 1. Enforcing the gates

Every claim this project makes is produced by scripts that a human runs. That is
the single largest gap: 184 tests and 13 bit-exact gates are worth exactly what
their enforcement is worth, and today that is "whoever remembered".

### 1.1 One job, everything

`cargo test --release --all-targets` then `run_script_parity.sh`, on
`ubuntu-latest`.

- **Pro** — one file, nothing to keep in sync; a red build means a real break; the
  workflow is a faithful copy of the local ritual, so "works on my machine" and
  "works in CI" cannot disagree.
- **Con** — ~10 min per push from cold (2.5 min build + 1 min staging + 6 min
  gates). Every commit pays the C++ cost even when it touches nothing but a
  comment; the first thing a contributor sees fail is often *staging*, not their
  change.
- **Risk** — the oracle is the fragile part: it needs the MSVC-isms patched, a
  specific `g++` invocation and the submodules checked out. If it breaks for
  environmental reasons, the job goes red for everything and people learn to
  ignore it within a week.
- **Outcome** — everything enforced, at a latency that discourages pushing often,
  and with a failure signal that mixes "your change is wrong" with "CI needs
  fixing".

### 1.2 Split: fast tests on every push, gates in a second job

Job `rust` (`cargo test`, no submodules, no `g++`) and job `parity` (stage the
reference, run the suite), parallel, `parity` unblocked from `rust`.

- **Pro** — the common regression (a Rust test) is visible in ~4 min including
  build; the two jobs fail for different reasons and the name says which; the
  fast job also works on Windows and macOS, where the oracle does not.
- **Con** — two jobs to maintain, and a duplicated release build unless the target
  cache is shared; contributors must read two check rows.
- **Risk** — the slow job gets treated as optional. That is a social risk, not a
  technical one, and it is exactly how nightly-only gates die.
- **Outcome** — near-total enforcement with a usable signal, provided the slow job
  is required for merge rather than advisory.

### 1.3 Recorded-fixture CI only (no C++ at all)

The `tests/*.rs` files already hold every live gate's *recorded* expectation
(`main_engine.trace`, `.resolved`, `.graph`, `.values`, `.execute`, `.errors`).
Run those in CI and keep the live oracle comparison as a manual ritual.

- **Pro** — minutes, no submodules, no `g++`, no MSVC patches; runs on any OS; the
  recorded files catch a port that drifts *away* from a known-good answer.
- **Con** — it cannot catch a port that drifts *with* its fixtures: re-record from
  a broken build and the CI is green and wrong. It also stops testing the only
  thing that makes the bit-exact claims meaningful — agreement with the original.
- **Risk** — the classic self-fulfilling gate. Fixtures become the truth instead
  of a memory of the truth.
- **Outcome** — a cheap tripwire that is strictly weaker, and dangerous if it is
  mistaken for the real thing.

### 1.4 Nightly live gates, PR fast tests

`rust` tests on every push; the oracle suite on a schedule and on `master`.

- **Pro** — PR latency drops to minutes; the expensive answer still exists; a
  regression is caught within a day on `master`.
- **Con** — a PR can merge broken and be repaired later; on a single-maintainer
  repository "nightly" is a thing nobody opens.
- **Risk** — schedule-only jobs rot silently: the first time it goes red is months
  later, and by then the fix is archaeology.
- **Outcome** — appropriate for a team with a dashboard, weak here.

### 1.5 Cache the staged reference and the target directory (a modifier)

`actions/cache` keyed on the submodule SHA + `Cargo.lock`, restoring `/tmp/ref*`
and `rust/target`.

- **Pro** — cuts ~60 s of staging and most of the build on repeat runs; orthogonal
  to 1.1–1.4, so it can be added to any of them.
- **Con** — caches hide a broken cold path; a stale cache can mask a staging
  script that no longer works.
- **Risk** — cache invalidation subtleties and size limits; a poisoned cache is
  harder to diagnose than a slow build.
- **Outcome** — a real speedup that should always be paired with one uncached job
  (e.g. on `master`) so the cold path is still exercised.

### 1.6 No CI; a `pre-push` hook or a documented ritual

- **Pro** — zero infrastructure.
- **Con** — that is what exists now, and what exists now has produced three
  instrument bugs found by hand (`measuring the process instead of the loop`,
  `frames that are timesteps`, `pipefail` swallowing a match), all of which a
  mechanical re-run would have surfaced earlier.
- **Risk** — the project's premise is "verified by gates"; unenforced gates are a
  claim about the author's memory.
- **Outcome** — rejected.

### What the contest shows

The options are framed as *how long* the CI takes, but the real axis is **what
kind of question each job answers**:

- "does the Rust tree still satisfy its own tests?" — no oracle needed, any OS,
  cheap, and it catches most mistakes;
- "does the Rust tree still agree with the original, bit for bit?" — needs the
  oracle, one OS, expensive, and it is the only thing that justifies the word
  *exact*.

Every cheap option is cheap by dropping the second question. Every expensive
option pays for it on every push. The mistake to avoid is choosing one; they are
two jobs with two failure modes, and the cost belongs to the one that earns it.

### Better way forward

1. **Two jobs, split by question, not by time.** `rust` (tests, no submodules, no
   `g++`) on `ubuntu-latest` + matrix `windows-latest` — the first time the port is
   proven to build where the original lives. `parity` (submodules, staged
   reference, `run_script_parity.sh`) on `ubuntu-latest`, required for merge.
2. **Make the suite partitionable** before it grows again: `run_script_parity.sh
   --only DRIVE,RUNNING` so a PR can run the gates its diff touches, while
   `master` runs all thirteen. This is the change that keeps item 3 affordable.
3. **Cache the build, never the oracle result**: cache `rust/target` and the
   staged `libref*.a`, keyed on `Cargo.lock` and the piranha commit; keep one
   nightly *cold* run so the staging path cannot rot.
4. **Guard the fixtures.** A tiny job that fails if a PR touches
   `tools/parity/**/*.{trace,graph,values,execute,engine,errors}` without also
   touching the code that produces it, and prints the diff. That closes 1.3's
   self-fulfilling hazard even though we do not adopt 1.3.
5. **Pin the toolchain** in `rust-toolchain.toml` (1.88.0, the version everything
   above was measured with) so CI, sandbox and contributors agree.
6. `measure_performance.sh` stays out of CI by design — a wall-clock number on a
   shared runner is noise — but it becomes a nightly job that *reports* (no
   failure threshold) so the real-time factor has a history.

Cost: half a day, mostly YAML. Value: everything else in this document stops
depending on a human.

---

## 2. The application shell

The last ⬜ row. What is missing is the window, the renderer, the UI, the audio
device and the input, i.e. `EngineSimApplication` and `delta-studio`. What is
*already* ported and gated is the part people assume is the hard part: the frame
loop's policy (starter release, ignition enable, throttle, gear, clutch), the
latency controller that scales a frame by 1.1/0.9, and the whole audio chain.

### 2.1 Full windowed application in Rust

`winit` + `wgpu`/`glow` + `egui` + `cpal`, reproducing the original's layout.

- **Pro** — the actual deliverable; cross-platform; the only option that gives a
  user what the original gives them.
- **Con** — an estimated 2–3 k lines of portable logic plus an entirely new
  graphics stack; assessment §9 puts the Windows/D3D glue at ~35 k LOC that must
  be rewritten rather than translated, and none of it exists here.
- **Risk** — **unverifiable in this environment**: no display, no GPU, no audio
  device, no target to compare against. The project's entire method — a gate that
  compares against the original — has nothing to grip. A shell written blind is
  the first component in the port that would ship on "it compiles".
- **Outcome** — a large, plausible-looking, unverified surface; the one thing this
  project has consistently refused to do.

### 2.2 "The application minus the window"

Port the shell's *decidable* half: the settings file, the input→action mapping,
the audio block scheduler and latency steering, behind traits with a null backend.

- **Pro** — everything except drawing is decidable and gateable against the
  original: `application_settings` is a file format (byte-compare), the latency
  controller is a numeric sequence (compare the frame's step counts), the input
  mapping is a table (compare state transitions), the audio sink is a trait
  (`--null-device` in CI).
- **Con** — no window, so "the app" still is not runnable for a user; and the
  original's threading (audio thread, input thread) is deliberately not
  reproduced, so "identical" needs care: it is identical *in output*, not in
  scheduling.
- **Risk** — scope creep: "just one small window" is how this option becomes 2.1
  by accident.
- **Outcome** — M7's ⬜ becomes "the application's behaviour, minus its pixels",
  with gates, and the remainder is a well-defined, small job.

### 2.3 Declare the shell out of scope

Finish as a library + CLI + instrument suite; document that the shell is not
ported and why.

- **Pro** — zero new risk; everything shipped stays verified; the honest version
  of "we ported the simulator, not the application".
- **Con** — the port never runs as an application; the ⬜ row stays ⬜; the
  assessment's own framing ("same sound") is not delivered.
- **Risk** — a user expecting a product finds a toolbox; the decision is invisible
  unless written down loudly.
- **Outcome** — maximal verification, minimal product.

### 2.4 A thin shell: audio device + text/console control surface

`engine-sim-play` crate: `cpal` output, keyboard/text input, the CLI's frame loop
paced by a clock, minimal readouts.

- **Pro** — a genuinely usable thing in ~300–500 lines: you can start an engine,
  open the throttle, shift, and hear it in real time; reuses everything already
  gated; the device is behind a trait so CI uses a null sink and compares the
  rendered block stream to `engine-sim-cli --wav`.
- **Con** — not the original UI: no gauges, no plots, no settings screen; and
  `cpal` breaks the "dependency-free" property, so it must be its own crate.
- **Risk** — audio device availability/behaviour differs per platform (buffer
  underruns become a *sound* problem, not a numeric one); CI cannot test the
  device path, only the trait.
- **Outcome** — the first thing in this project a non-developer can run, gated
  everywhere except the last inch.

### 2.5 Web shell (wasm + WebAudio + canvas)

- **Pro** — runs anywhere, demoable in a browser (including in this preview
  environment), no platform audio code; wasm is IEEE-754, so the arithmetic stays
  identical.
- **Con** — the convolution is 10 000 taps per channel per sample, and the port
  already sits at 1.0–1.3 x real time natively; wasm typically costs another 2–5 x.
- **Risk** — it would very likely land *below* real time, which fails the one hard
  constraint the application has; and it would need its own parity gate to be
  believable.
- **Outcome** — an attractive demo that probably cannot hold a note.

### 2.6 Keep the C++ application, link the Rust core

Ship the existing app with `engine-sim-core` behind a small C ABI.

- **Pro** — users keep the UI they know; the Rust core is already proven; the FFI
  surface is small (`Simulator` + one script entry point).
- **Con** — two languages in one binary; not the stated goal; handle/lifetime
  design at the boundary (Rust `Vec`s, the script graph) invites UB that no gate
  would catch.
- **Risk** — an FFI bug is a crash class this project has never had.
- **Outcome** — a pragmatic product path, not a port.

### What the contest shows

2.1 and 2.5 both fail the same way: they are *unverifiable here*, so they trade
the project's only real asset (evidence) for surface area. 2.3 refuses to
disappoint anyone by refusing to try. 2.4 and 2.2 are the same insight from two
directions: **the shell's behaviour is port-able and gateable; the shell's
rendering is not.** And 2.6 shows the product question and the port question are
different questions.

The decomposition that makes this tractable is not "how much of the shell" but
**"which layer"**: policy (gateable today), device (gateable behind a trait),
pixels (not gateable here at all).

### Better way forward

Three phases, each with its own gate, stopping being a decision rather than an
accident:

- **2a — the shell's decisions.** Note (corrected after reading the code): there
  is no settings *file* to port — `ApplicationSettings` is written by the
  **script** (`set_application_settings`, already compared by the `SETTINGS` gate
  for all 40 scripts) and any persistence lives in `delta-studio`. What is missing
  is `configure()`'s derived state (eleven `srgbiToLinear` colours, five unit
  strings), the input→action table (`src/engine_sim_application.cpp`, lines
  ~206–800), and the latency controller as an object you can assert on (its output
  already appears in the traces as `frame_steps`). Portable, dependency-free, and
  it closes most of the row's *meaning*.
- **2b — a device, not a window.** `engine-sim-play`: `cpal` sink + keyboard, the
  CLI loop paced by a clock, `--null-device` for CI where the rendered stream must
  equal `engine-sim-cli`'s byte for byte. This is the deliverable a person can
  actually use, and the only unverified inch is the OS buffer.
- **2c — pixels, only if wanted.** `winit` + `egui`, the *minimum* surface (open a
  script, throttle, gear, rpm/speed, start/stop) — explicitly **not** a port of
  the original's UI, and documented as a new front end rather than a ported one.
  Everything it displays already comes from a gated source.

What I would not do: attempt the original's UI as a port. Its 35 k LOC of
platform glue is the least verifiable, least portable, most environment-bound part
of the whole project, and reproducing it would put the project's first unverified
component in the place where users look first.

---

## 3. Drivetrain breadth

`DRIVE` drives three vehicles (the TRX520's, and the two hand-built fixtures'
fallbacks). All 20 engine assets publish their own `vehicle(...)` and
`transmission(...)`, and their *parameters* are already compared by the `OBJECTS`
gate for all 40 scripts. What is not compared is **behaviour**: that a given mass,
diff ratio, tyre radius and gear set actually move a car the way the original
does.

### 3.1 Extend `DRIVE` to all 20 engines

- **Pro** — closes the hole directly and in the place people already run; ~2.3 s
  per engine measured, so +45–60 s.
- **Con** — 20 000 frames × ~40 fields is a 764 000-line trace (~30 MB) per side,
  so it is 60 MB written and diffed per engine, ~1.2 GB of transient IO per run.
- **Risk** — the suite is already 5 m 50 s and growing; every addition makes the
  human step more tempting to skip — which is item 1's problem.
- **Outcome** — coverage, bought with suite time that will keep scaling with the
  number of assets.

### 3.2 Separate `drive_all.sh` sweep, outside the gate suite

- **Pro** — keeps the suite fast; can afford longer runs (a real 2 s of engine
  time each).
- **Con** — unenforced by default, which is the exact failure mode item 1 exists
  to fix.
- **Risk** — rots; the first run after six months fails for unrelated reasons.
- **Outcome** — acceptable only once CI runs it on a schedule and someone reads
  the result.

### 3.3 Reduce what a sweep traces

Compare a drivetrain field set (`crank_v_theta`, `filtered_speed`, `dyno_torque`,
`frame_steps`) instead of all ~40 fields.

- **Pro** — 6–7 x smaller traces, and a drivetrain failure is easier to read in 4
  columns than in 40. (Corrected: this is a *readability* win, not a cost win — a
  764 000-line trace is written and diffed in ~0.3 s, while the simulation behind
  it takes seconds. Breadth is bought with the engine-time rule, not with fewer
  fields.)
- **Con** — a field left out is a blind spot, and the audio fields are exactly
  what catches subtler divergence.
- **Risk** — someone later assumes the sweep covers everything `DRIVE` covers.
- **Outcome** — better signal per byte, at the price of a new mode in both runners
  and an explicit statement of what is *not* compared.

### 3.4 Fixed engine time instead of fixed frames

`DRIVE`/`DRIVEN` still use 20 000 *frames*, which is one simulation step each —
0.5 s on the 40 kHz TRX520 and 4 s on the 5 kHz 412 T2. Same trap `RUNNING` had.
`DRIVEN` adds a second one: `main.mr` runs at 20 kHz, so its window is exactly
1.0 s and the starter is released at 1.0 s — the last frame. Measured, it fires at
frame 4 649 and reaches 4 567 rpm, but always while *overpowering a connected
starter*; the gate has never traced an engine running free of one. The recipe
needs a release it can reach, not just a new frame count.

- **Pro** — makes "two seconds of driving" true for every engine; costs nothing
  (same total); removes an inconsistency that has already produced one wrong
  report.
- **Con** — no new *coverage* on its own; and the numbers in existing docs/records
  move.
- **Risk** — low; the traces are live, not recorded, so nothing needs re-recording.
- **Outcome** — the claim becomes true. Do it regardless of the other choices.

### 3.5 Unit-level drivetrain parity

Drive `Vehicle`, `Transmission`, `Dyno`, `Starter` and the clutch constraints
directly with a table of inputs, both sides, no script and no engine.

- **Pro** — microseconds per case, so thousands of cases: gear × clutch × torque ×
  speed × mass, including the singular corners (zero speed, locked clutch,
  reverse); failures point at a formula rather than at a frame number.
- **Con** — it does not test the wiring from a script's object to the simulator,
  which is where two real bugs have already lived (`ADD_SLAVE_JOURNAL`, the
  exhaust index).
- **Risk** — duplicated coverage invites the assumption that one of them is
  redundant.
- **Outcome** — a sharp, cheap instrument that complements rather than replaces
  the script-level gate.

### 3.6 Sample six topologies

Drive six representative cars (single, twin, 60° V6, 90° V8, V12, radial 9).

- **Pro** — a third of the cost of 3.1; covers every layout in the corpus; the
  drivetrain code does not know how many cylinders exist.
- **Con** — "representative" is a judgement, and the *parameters* (mass, diff,
  ratios) vary per asset, not per layout.
- **Risk** — a bad vehicle in an unsampled asset goes unnoticed — though its
  numbers are already gated by `OBJECTS`.
- **Outcome** — reasonable cost/benefit, but it answers "does the code work" and
  not "does asset 14's car work".

### What the contest shows

The question "should all 20 be driven" hides two different ones: **are the
drivetrain's formulas right** (a property of ~600 lines of Rust, best tested in
isolation, thousands of cases, microseconds each) and **does this asset's car
behave** (a property of 20 parameter sets, best tested end-to-end, once each).
3.1 answers the second at the price of the first being answered 20 times over; 3.5
answers the first far better and the second not at all; 3.3 and 3.4 are what make
the second affordable.

### Better way forward

1. **3.4 first, as a correctness fix**: `DRIVE`/`DRIVEN` take a duration
   (`frames = simulation_frequency * seconds`), the same rule `RUNNING` now uses.
   Cheap, and it makes every subsequent number honest.
2. **Answer the formula question with 3.5**: a `drivetrain_parity` harness driving
   the classes directly through a table — including the corners a script will
   never reach (locked clutch at zero speed, reverse, dyno hold at both ends).
   This is where new *bugs* are likely; the script drives are for *wiring*.
3. **Answer the wiring question with 3.3 + 3.1, at 0.5 s per asset**: every engine
   driven through its own gearbox for half a second of engine time, comparing the
   drivetrain field set. ~0.3–1 s per engine, so all 20 for well under a minute,
   in the gate suite, alongside the three deep 2 s drives that keep the full field
   set.
4. Keep the three deep drives exactly as they are: they are the ones that would
   catch a field the sweep does not look at.

That dissolves the trade-off instead of picking a side of it: depth stays where
it is, breadth becomes cheap enough to be unconditional, and the formulas get a
harness with better reach than any script.

---

## 4. Order, and what I would not do

1. **CI (item 1)** — first, because it is the only change that makes every other
   claim checkable, and because items 2 and 3 both *add* gates. Half a day.
2. **Drivetrain (item 3)** — the correctness fix (3.4), then the unit harness
   (3.5), then the cheap all-20 sweep (3.3 + 3.1). A day or two.
3. **Shell 2a** (settings, input table, latency controller) — a day, all gated.
4. **Shell 2b** (`engine-sim-play`, null-device gate) — a day, and the first
   version a person can actually use.
5. **Decide 2c** (a window) only after 2b exists, and only as a new front end.

Not doing, with reasons recorded:
- porting the original's UI (~35 k LOC of unverifiable platform glue);
- a wasm demo (fails real time by construction, at 10 000 taps a sample);
- reusing 1.3's fixture-only CI as if it were the real gate;
- adding all-20 drives *without* the field reduction and the engine-time fix, which
  would buy coverage by making the suite slower than anyone will run.

## 5. Acceptance criteria

| piece | done when |
|---|---|
| CI | a PR shows `rust` (tests, Linux + Windows) and `parity` (all 13 gates, Linux); both required; a fixture-only change is flagged |
| drivetrain unit harness | ≥ 1 000 cases across gear/clutch/torque/speed, bit-exact, under a second, in `cargo test` |
| drivetrain sweep | all 20 assets, 0.5 s of engine time each, drivetrain fields identical, under a minute, in the gate suite |
| `DRIVE`/`DRIVEN` | take seconds, not frames; the docs' numbers restate what the run is |
| shell 2a | settings file byte-identical; step-count sequence identical for a fixed `dt` sequence; input table identical |
| shell 2b | `--null-device` output byte-identical to `engine-sim-cli --wav`; audible on Linux with a real device |
| shell 2c | a window that draws only values from gated sources; explicitly not a port of the original's UI |
