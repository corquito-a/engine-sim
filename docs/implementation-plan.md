# Implementation plan: CI, drivetrain breadth, and the shell

This turns [`next-steps.md`](next-steps.md) into work items: what changes, in which
file, how it is verified, and what "done" means. Order: CI first (it makes
everything after it checkable), then the drivetrain, then the shell's decidable
half.

Effort figures are working sessions on a 2-vCPU box, not calendar days; the
verification commands are the same ones a contributor would run.

## 0. Corrections found while planning

Five things in the options paper were wrong or undersold once the code was
re-read. They change the plan, so they come first.

| # | what I assumed | what is true | effect |
|---|---|---|---|
| C1 | the shell needs a *settings file* ported | `ApplicationSettings` is produced by the **script** (`set_application_settings` → `objects.rs`, already compared by the `SETTINGS` gate for all 40 scripts). There is no settings file in this repo; persistence, if any, lives in `delta-studio` (a submodule, not checked out). | 2a shrinks to `configure()`'s pure effects — `srgbiToLinear` colour conversion plus the unit strings — and the input table. Small, and no new format to invent. |
| C2 | driving all 20 engines costs 3–5 minutes | a frame is ~30 µs and 0.5 s of engine time is `frequency / 2` frames; summed over the corpus that is ~160 k frames ≈ **20 s** for both sides together | the all-20 sweep is cheap enough to be unconditional |
| C3 | a reduced field set is what makes breadth affordable | the cost is simulation, not the diff (764 k lines ≈ 30 MB is written and diffed in ~0.3 s) | field reduction is a *readability* option, not a prerequisite; buy breadth with the engine-time rule instead |
| C4 | `DRIVEN` traces a running engine | it runs 20 000 frames = 1.0 s at `main.mr`'s 20 kHz, and releases the starter at *exactly* 1.0 s — the last frame. The engine does fire (first `chamber_lit` at frame 4 649) and reaches 4 567 rpm, but it is overpowering a connected starter, never running free of one | `DRIVEN`'s recipe needs a starter release it can actually reach, not just a new frame count |
| C5 | the parity script would run in CI as-is | `run_script_parity.sh` prepends `PATH=/tmp/rust-toolchain/bin` unconditionally and takes no gate filter | two small edits before CI can use it (Phase 0) |

Everything else in `next-steps.md` stands.

**Item 4 (performance) is investigated separately**:
[`performance-investigation.md`](performance-investigation.md) (the profile) and
[`performance-options.md`](performance-options.md) (the contested menu). Three of
its findings have **landed** on this branch, before Phase 1, because every phase
below runs the front end and the audio path many times:

* `55b0983` — the expansion memo is bucketed by a context chain hash. `radial_9`'s
  front end went from 4 115 ms to **747 ms**; the gate suite from **350 s to 280 s**.
* `80e79e7` — one chamber volume per timestep instead of nine reads.
* `0ea3870` — two samples per pass on the mono convolution path: **−14.3%** of a
  `main.mr` frame, **−28.9%** of a `trx520` frame.

All three are bit-identical on every check available (the 13 oracle gates included)
and none touches a formula. **The cost table below was measured before them**, so
Phase 0's estimates are pessimistic by ~20% on anything that starts the front end;
re-measure rather than re-estimate when the phase is picked up. The rest of the
investigation's list (O5 `build_graph`, O2b the stereo layout) is measurement work,
not code, and can wait.

---

## Phase 0 — make the suite programmable (½ session)

**Goal:** the gate suite can be *asked a subset of questions*, and it runs
wherever a compiler and a device-free shell exist. Nothing in CI is possible
before this.

### Tasks

1. **`run_script_parity.sh`: gate filter.** Wrap each gate block in a function
   named for its label — `gate_ir`, `gate_run`, `gate_driven`, `gate_drive`,
   `gate_running`, `gate_graph`, `gate_values`, `gate_objects`, `gate_engine`,
   `gate_optimize`, `gate_wave`, `gate_engines`, `gate_execute` — and drive them
   from a list:
   ```bash
   ONLY="${ONLY:-}"                     # e.g. --only DRIVE,RUNNING
   want() { [ -z "$ONLY" ] || [[ ",$ONLY," == *",$1,"* ]]; }
   for gate in IR RUN DRIVEN DRIVE RUNNING GRAPH VALUES OBJECTS ENGINE OPTIMIZE WAVE ENGINES EXECUTE; do
       want "$gate" && "gate_${gate,,}"
   done
   ```
   Keep the printed lines byte-identical so existing logs stay comparable, and add
   `--list` printing the labels. **Acceptance:** `--only EXECUTE` runs one gate in
   under 2 s; a full run produces the same output as today.
2. **Toolchain discovery instead of assumption.** `[ -d /tmp/rust-toolchain/bin ] &&
   export PATH="/tmp/rust-toolchain/bin:$PATH"`, and the same for the reference
   prefix (`REF_SCRIPT` — already overridable). **Acceptance:** the script runs
   with a system toolchain in `PATH` and no `/tmp/rust-toolchain`.
3. **`rust/rust-toolchain.toml`** pinning `1.88.0` (the version every number in
   these documents was measured with). Harmless for the sandbox's hand-built
   toolchain, authoritative for rustup users and CI.
4. **`engine_frames <script> <seconds>` helper**, lifted from `RUNNING`'s inline
   `graph_dump --engine` + python decode, so `DRIVE`, `DRIVEN` and `RUNNING` all
   ask in seconds. (The decode is a hex-float; python3 is already a dependency of
   the harness, so it stays a one-liner.)
5. **`docs/testing-strategy.md`**: one paragraph on why the suite is partitionable
   (PRs run what they touch; `master` runs everything) — the same argument as
   "a measurement needs the same proof as a gate".

**Risk:** refactoring 13 sequential blocks into functions can silently change
`set -euo pipefail` behaviour (a failing gate inside a function still exits the
script — verify with an intentionally broken gate). Mitigation: after the
refactor, run the suite once and diff the output against a captured baseline.

---

## Phase 1 — CI (½ session)

**Goal:** two required checks, split by the question they answer, not by their
runtime.

### Tasks

1. **`.github/workflows/rust.yml`**, three jobs:
   - `tests` — `ubuntu-latest`, `actions/checkout@v4`,
     `Swatinem/rust-cache@v2`, `cargo test --release --all-targets`. No
     submodules, no `g++`.
   - `tests-windows` — same on `windows-latest`. This is a **discovery** step: no
     crate contains any platform-specific code (`grep -rn "cfg(target_os\|cfg(unix\|cfg(windows"`
     in `rust/crates` returns nothing), so it should pass; if it does not, the
     failure is worth knowing and the job lands as advisory with the failure
     quoted in the PR.
   - `parity` — `ubuntu-latest`, `git submodule update --init --recursive
     dependencies/submodules/piranha` (10 s, 2 MB), cache `/tmp/refscript` keyed on
     `git rev-parse HEAD:dependencies/submodules/piranha` + toolchain version,
     then `bash rust/tools/parity/run_script_parity.sh`. Required for merge.
2. **Publish measurements as job summaries**: the parity job writes the gate
   lines (`PASS: 20 engines, every checkpoint identical; 13 of them catch …`) into
   `$GITHUB_STEP_SUMMARY`, so the numbers are visible on the PR rather than in a
   log nobody opens.
3. **Nightly cold run** (`schedule: cron`) of `parity` with the caches disabled, so
   the staging path cannot rot behind a warm cache.
4. **Fixture visibility, not enforcement**: a step that prints `git diff --stat`
   for `tools/parity/**` and `crates/*/fixtures/**`. Enforcement is unnecessary
   because the live gates compare against the *oracle*, not the fixtures — a
   re-recorded fixture cannot make `parity` pass. Write that invariant down in
   `docs/testing-strategy.md`; it is the reason fixture-only CI was rejected.
5. **`measure_performance.sh` nightly, reporting only** (no threshold), so the
   real-time factor has a history; a wall clock on a shared runner is not a gate.
6. **README badge + a "how CI is split" paragraph** in `rust/README.md`.

**Verification:** open a throwaway PR with a deliberate one-line change to
`values.rs`; `tests` must go red in ~4 min, `parity` in ~10 min. Remove it.

**Acceptance:** `tests` and `parity` required and green on `master`; cold `parity`
≤ 12 min, warm ≤ 8 min; nightly green.

**Risk:** the Windows job could arrive red. Mitigation: it is scoped as discovery,
and the PR either fixes it or parks it explicitly — no silent `continue-on-error`.

---

## Phase 2 — drivetrain (1–2 sessions)

**Goal:** the drivetrain's *formulas* get a harness that can reach corners no
script reaches, and the *parameters* of all 20 assets get driven once each, for
about 20 s of suite time.

### 2.1 The recipe fix (first, because every other number depends on it)

- `DRIVE` and `DRIVEN` take **seconds**: `frames=$(engine_frames "$script" 2.0)`,
  and the starter is released at `0.25 × seconds` rather than 1.0 s (C4 makes this
  necessary: at 20 kHz a 1.0 s release is the last frame of the run).
- Update the three documents that quote the old numbers (`rust/README.md`,
  `docs/mr-interpreter-plan.md` §8.7/§8.8, `docs/testing-strategy.md`), and state
  the window in the gate's own output: `PASS: … , 2.00 s of engine time`.
- `tests/run_tests.rs` and `fixtures/main_engine.trace` are a *separate* 400-frame
  recording and are unaffected.

**Acceptance:** `DRIVEN` reports a starter release it reaches; the trace shows the
engine running free after it; both sides still identical.

### 2.2 Unit-level drivetrain parity (the new instrument)

Where the bugs will be: the classes, not the wiring.

- New oracle mode `drivetrain_reference.cpp` (mirrors `core_reference.cpp`'s
  shape): build `Vehicle`, `Transmission`, `Dyno`, `StarterMotor`,
  `TransmissionClutch` on `atg_scs`, drive a table of cases, print one row per
  step with the five fields that carry the drivetrain's behaviour
  (`crank_omega`, `wheel_omega`, `vehicle_speed`, `clutch_torque`, `dyno_torque`).
- New Rust example `drivetrain_trace.rs` emitting the same rows from
  `engine-sim-core`'s classes directly — no script, no engine.
- Cases (a table in both, generated from the same source of truth so they cannot
  drift): gear 1–6 and neutral and reverse × clutch pressure 0, 0.25, 0.5, 1.0 ×
  vehicle mass 500/1200/2500 kg × diff 2.7/3.9/4.8 × imposed crank speed
  0/500/3 000/7 000 rpm, plus the corners: locked clutch at zero speed, dyno hold
  at its min and max speed, and a shift while the clutch is engaged.
- Gate `DRIVETRAIN` in the suite (one comparison of the whole table, a few seconds)
  plus `tests/drivetrain_parity.rs` holding a recorded fixture so it also runs
  without the oracle.

**Acceptance:** ≥ 1 000 cases, bit-exact, under a second of wall clock, and it
fails when a single coefficient in `Transmission::apply` is perturbed by 1 ulp.

### 2.3 Drive all 20 assets, cheaply

- New gate `DRIVE_ALL`: for every wrapper, `frames = engine_frames <script> 0.5`,
  `PARITY_GEAR=1 PARITY_CLUTCH=1 PARITY_THROTTLE=1`, compare the full trace
  (C2/C3: ~20 s total, no field filtering needed).
- Keep the three deep 2 s drives exactly as they are — they are what would catch a
  field the sweep does not look at, and they are cheap.
- Report per engine like `RUNNING` does, so a single asset's failure names itself.

**Acceptance:** 20 assets driven, suite ≤ 7 min total, and a deliberately wrong
`diff_ratio` in one asset's wrapper fails only that asset.

---

## Phase 3 — the shell's decidable half (1 session)

**Goal:** everything about `EngineSimApplication` that is not drawing becomes a
gated crate.

### Tasks

1. **New crate `rust/crates/engine-sim-app`** (dependency-free, like the others):
   - `settings.rs` — `ApplicationSettings` exactly as the script writes it, plus
     `configure()`'s derived state: the eleven `srgbiToLinear` colours and the five
     unit strings (C1). `srgbiToLinear` is a 3-line formula; it becomes a unit
     test with values taken from the original.
   - `input.rs` — the key→action table read out of
     `src/engine_sim_application.cpp` (lines ~206–800): `1`–`5` units, `F1`–`F3`,
     `Tab`, `F`, `Insert`, `Escape`, `Return`, `Right` (pause/step), `Space`
     (fine control), `Z/X/C/V/B/N` (gears), `G` (dyno hold), `Q/W/E/R`
     (throttle/starter/ignition per the file). A pure state machine
     `InputState { pressed, held, released } → Action`.
   - `pacing.rs` — the latency controller as its own object: given a frame `dt` and
     the synthesizer's latency target, produce the step count the frame gets
     (the 1.1 / 0.9 steering the CLI already reproduces through
     `Simulator::start_frame`). One function, side-effect free, so it can be
     asserted rather than inferred.
2. **Oracle support** — extend `script_reference` with `--app-state`: run the
   script, then print `configure()`'s outputs (the eleven linear colours, five unit
   strings) so `graph_dump --app-state` can be compared. This is the gate that
   turns "the script wrote settings" into "the application would look like this".
3. **Characterisation tests** for the input table: it has no oracle (the mapping is
   in C++ code, not data), so the test pins the table with the source file and line
   for each row — a transcription test, in the spirit of the earlier
   `butterworth_coefficients_match_the_reference` test. State plainly in the test
   doc comment that this is *not* a parity gate.
4. **Wire the CLI through the new crate** (`--settings` uses `settings.rs`), so the
   shell's state has exactly one definition and the CLI is what proves it.

**Acceptance:** `APPSTATE` gate green for all 40 scripts; the input table test
covers every key the application reads; no new dependency.

---

## Phase 4 — a device, not a window (1–2 sessions)

**Goal:** something a person can run and hear, with the untestable part reduced to
one thin adapter.

### Constraints that shape it

- **This sandbox cannot fetch crates** (only GitHub and package registries for
  other ecosystems are reachable), so `cpal` is not available here, and there is
  no audio device in the build environment either. Anything device-shaped must be
  written against a trait, compiled behind a feature, and left deliberately
  unexercised — with that fact recorded, not glossed.

### Tasks

1. **`rust/crates/engine-sim-play`** (may depend on `engine-sim-cli`'s pieces):
   - `Sink` trait: `write(&[i16]) -> usize`, `latency_samples()`, `close()`.
   - `NullSink` — renders as fast as possible; the CI path.
   - `PacedSink` — a real-time clock (sleep-until-next-deadline) feeding `NullSink`;
     this is what makes `--pacing` meaningful in a device-free environment.
   - `AlsaSink` behind `--features alsa`: `extern "C"` declarations for
     `snd_pcm_open/writei/set_params` and `#[link(name = "asound")]`, ~150 lines,
     **untested here** — the plan ships it as opt-in and says so in its doc
     comment, rather than pretending.
   - Keyboard input: a `BTreeMap<char, Action>` driven by stdin (no `termios`
     crate; `stty` via `std::process::Command` where available, raw-mode documented).
2. **Gate `PLAY`**: `engine-sim-play --null-device --seconds 2 --wav out.wav` must
   produce a file byte-identical to `engine-sim-cli --seconds 2 --wav`: the same
   audio through a different front end, which proves the shell's path is the
   simulator and nothing else.
3. **`--pacing` reports**: frames rendered, wall time, real-time factor, dropped
   blocks. Reported, never asserted — the same rule as `measure_performance.sh`.
4. **README section** with a recording-free "how to run it" and the exact limits:
   Linux+ALSA tested or not, Windows absent, GUI absent.

**Acceptance:** `PLAY` gate green; `cargo run -p engine-sim-play -- --null-device
--seconds 2` byte-identical to the CLI; the ALSA path documented as unverified.

---

## Phase 5 — decision point: pixels

Not scheduled. It starts only when Phases 1–4 are merged, and only with an answer
to one question: **is a new minimal front end acceptable, given it is not a port of
the original's UI?**

If yes: `winit` + `egui`, one screen — open a script, throttle, gear, rpm, speed,
start/stop — drawing only values that already come from gated sources, in its own
crate, tested as far as a headless environment allows (frame-graph construction,
state transitions, and a smoke test under `xvfb`).

If no: record the decision in `rust/README.md`'s status table (the ⬜ row becomes
"not ported, and why") and stop. Both are honest; drifting into a half-written
window is not.

---

## Order, budget, and the rules that keep it honest

| phase | delivers | suite cost | effort |
|---|---|---|---|
| 0 | a suite that can be asked a subset | 0 | ½ |
| 1 | `tests` + `parity` required; nightly; performance history | 0 | ½ |
| 2 | `DRIVETRAIN` (≥ 1 000 cases), `DRIVE_ALL` (20 assets), a starter release that happens | +25 s | 1–2 |
| 3 | `APPSTATE` gate, input table, pacing object | +5 s | 1 |
| 4 | `engine-sim-play`, `PLAY` gate, a device adapter | +5 s | 1–2 |
| 5 | a window, or a recorded decision | — | ? |

**Rules**

1. One PR per phase, merged only with `tests` and `parity` green.
2. The suite must stay **≤ 7 minutes**. Anything that would push past it is
   partitioned (Phase 0) rather than dropped or hidden behind `--only` on master.
3. A gate added in a phase comes with its recorded fixture, so the Rust-only job
   can still see it.
4. Any re-recorded fixture is re-recorded from the **oracle**, in the same PR, with
   the diff visible.
5. Numbers in documents are corrected in the same PR that changes them — the
   `8 000 frames` episode cost a day and a half of confusion and is the reason this
   rule exists.

## Appendix: first session checklist

- [ ] `run_script_parity.sh`: gate functions + `--only` + `--list`; output diffed
      against a captured baseline; `--only EXECUTE` under 2 s.
- [ ] `run_script_parity.sh`: toolchain discovery instead of `/tmp` assumption.
- [ ] `rust/rust-toolchain.toml` (1.88.0).
- [ ] `engine_frames` helper; `RUNNING` uses it (no behaviour change).
- [ ] `.github/workflows/rust.yml`: `tests`, `tests-windows` (advisory),
      `parity` (required), nightly cold, performance report.
- [ ] Throwaway PR proves the two checks go red for the right reasons; deleted.
- [ ] `rust/README.md` + `docs/testing-strategy.md` describe the split.
