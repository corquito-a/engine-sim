# Engine Simulator — native Rust port

Incremental port of the Engine Simulator simulation core to Rust. The C++
tree at the repository root is untouched and remains the reference
implementation; every ported module is verified against it numerically.

Motivation and estimates: [`docs/rust-port-assessment.md`](../docs/rust-port-assessment.md).

---

## Status

| Module | Upstream | State | Verification |
|---|---|---|---|
| `constants` | `include/constants.h` | ✅ done | bit-exact |
| `units` | `include/units.h` | ✅ done | bit-exact |
| `utilities` | `include/utilities.h`, `src/utilities.cpp` | ✅ done | bit-exact |
| `gas` (`GasSystem`, `Mix`, `State`, `FlowParameters`) | `include/gas_system.h`, `src/gas_system.cpp` | ✅ done | bit-exact, 185 checkpoints |
| `function` (splines, gaussian/triangle filters, caches) | `include/function.h`, `src/function.cpp` | ✅ done | bit-exact (whole-core) |
| `parts` (crankshaft, banks, heads, valvetrain, camshaft, pistons, rods, intake, exhaust) | `include/*.h`, `src/*.cpp` | ✅ done | bit-exact (whole-core) |
| `fuel` (laminar flame speed, turbulence maps) | `include/fuel.h`, `src/fuel.cpp` | ✅ done | bit-exact (whole-core) |
| `chamber` (combustion, flame front, flow, friction) | `include/combustion_chamber.h`, `src/combustion_chamber.cpp` | ✅ done | bit-exact (whole-core) |
| `drivetrain` (dyno, starter, clutch, transmission, vehicle) | `src/dyno*`, `src/transmission*`, `src/vehicle.cpp` | ✅ done | bit-exact (whole-core, incl. dyno hold) |
| `ignition` | `include/ignition_module.h` | ✅ done | bit-exact (whole-core) |
| `engine` (graph wiring, throttle/governor, VTEC conditions) | `include/engine.h`, `src/engine.cpp` | ✅ done | bit-exact (whole-core) |
| `simulator` (`PistonEngineSimulator` step loop, audio tap) | `src/piston_engine_simulator.cpp` | ✅ core | bit-exact (whole-core) |
| `atg_scs` subset (rigid bodies, constraints, NSV solver, Gauss-Seidel) | `submodules/simple-2d-constraint-solver` | ✅ done | bit-exact (whole-core) |
| `dsp` (low pass, Butterworth f32/f64, derivative, delay, convolution, leveling, jitter, `<random>` compat) | `src/*_filter.cpp`, `delta-studio`'s `convolve.h` | ✅ done | bit-exact (filter harness) |
| `synth` (input resampling, impulse response, render/mix, int16 out) | `src/synthesizer.cpp` | ✅ done | bit-exact (filter harness + whole-core incl. audio) |
| `script` front end (`.mr` scanner, parser, IR) | `piranha/flex-bison/*`, `include/ir_*.h` | ✅ done | IR round trip through the original piranha objects, 80 scripts |
| `script` IR substrate (`lower` arena, `channel_type`, `error`) | `include/ir_*.h`, `channel_type`, `compilation_error` | ✅ done | every unit lowers and raises back field-for-field |
| `script` resolution (definitions, types, attributes) | `piranha/src/ir_*.cpp` | ✅ done | 660/660 + 701/701 rows identical to the reference |
| `script` circularity checks | `piranha/src/ir_*.cpp` | ✅ done | E0020 gated by fixtures |
| `script` validation (reported errors) | `piranha/src/ir_*.cpp` | ✅ done | 15 negative fixtures reproduce the reference's exact code multiset |
| `script` references / expansion / type checks / node graph | `piranha/src/ir_*.cpp`, `node_graph.cpp` | ✅ done | `GRAPH`, `VALUES`, `EXECUTE`, `OPTIMIZE` gates |
| `scripting/` bindings (`EngineContext` → `Engine`, vehicle, transmission) | `scripting/` | ✅ done | `RUN`, `DRIVEN`, `DRIVE` gates, 16 265 + 3 × ~800 000 checkpoints |
| `engine-sim-cli` (a script run without the application) | `EngineSimApplication`'s frame loop | ✅ done | the same drive as `DRIVEN`, 7 tests |
| breadth and performance (every engine driven; real time) | `EngineSimApplication` | ◐ done for the headless half | `RUNNING` (20 engines), `measure_performance.sh` (1.30x real time) |
| app shell (window/render/UI/audio out) | `delta-studio` | ⬜ | — |

**Verification result: the whole simulation core and the audio path are
bit-exact.** Running the same single-cylinder engine (the `01_honda_trx520.mr`
fixture) through the original C++ and through the Rust port for **200 000
physics steps (20 s of simulated time, 0.5 s per step)** produces **8 964 002
identical `f64` bit patterns** — crank/piston/rod kinematics, chamber
pressure/temperature/moles and mixture, intake/runner/exhaust pressures, dyno
torque, the synthesizer's resampled input, and every rendered `int16` audio
sample, at every step.

That run covers the parts that are easy to get *almost* right: ignition
timing, flame-front propagation and burn efficiency (including the fuel's
random efficiency term), gas exchange through the port flow functions, the
constraint solver's Gauss-Seidel iterations and its per-frame warm-started
`lambda`, and the frame-level bookkeeping in `PistonEngineSimulator`.

The earlier, per-module result stands as well: on 185 scripted gas-system
checkpoints covering thermodynamics, compression work, reactions, mixture
bookkeeping, choked and subcritical flow, momentum transfer, degrees of freedom
3/5/7 and pairwise equilibrium flow, the two implementations agree bit for bit.

---

## Fidelity rules

The simulator's product is engine *audio*, which emerges from thousands of
hand-tuned arithmetic operations with no specification behind them. The port is
therefore a transliteration, not a rewrite:

1. Operation order matches the C++ expression-for-expression.
2. No `mul_add` / FMA contraction, no fast-math.
3. `std::pow` → `powf`, `std::sqrt` → `sqrt`, `std::fmin/fmax` → `f64::min/max`.
4. Defaults are preserved (`Mix{0, 1, 0}`, `degreesOfFreedom = 5`).
5. C++ overloads resolved implicitly (default `Mix`, default `beta`) become
   explicit `_default` variants in Rust — e.g. `initialize_default`,
   `update_velocity_default`, `flow_env_default`.
6. Every function carries a comment naming its C++ original.

Where the C++ relies on object identity or iteration order, the port reproduces
that order instead of "improving" it — constraint registration order, `rand()`
draw order, array traversal order. Those are all load-bearing (see below).

Two shape changes are unavoidable:

* `GasSystem::flow`: C++ stores two `GasSystem *` in `FlowParameters`; Rust
  takes them as two `&mut` arguments (`GasSystem::flow(&mut a, &mut b,
  &params)`), keeping the aliasing rules while preserving the pointer-swap
  logic.
* The solver owns its constraints in a `Vec<Box<dyn Constraint>>` and refers to
  rigid bodies by index (`i32`, `-1` = unset) instead of by pointer.

## Building and testing

The sandbox has no rustup and no access to `static.rust-lang.org`, so a minimal
toolchain is bootstrapped from the official release tarballs republished on the
npm registry:

```bash
./tools/bootstrap-toolchain.sh                 # installs rustc + rust-std + cargo to /tmp/rust-toolchain
export PATH="/tmp/rust-toolchain/bin:$PATH"
export CARGO_HOME=/tmp/cargo-home
```

The crate has **no external dependencies**, so it builds offline once the
toolchain is present. It does use this workspace's own `engine-sim-core`, for the
two `es_script` classes whose value is a `GasSystem` calculation.

```bash
cd rust
cargo build --all-targets     # no warnings
cargo test                    # 183 tests (core unit + gas + end-to-end + script + CLI + parity gates)
cargo test --release
./tools/parity/run_core_parity.sh 200000   # 8 964 002 bit-identical checkpoints
```

`crates/engine-sim-core/fixtures/trx520.rs` is the shared fixture: it rebuilds
`assets/engines/atg-video-1/01_honda_trx520.mr` with the parameter values the
upstream scripting layer produces. It is `include!`d by the tests and examples
(not a Cargo target).

## The command line

`crates/engine-sim-cli` is the application's frame loop with the window removed:
a `.mr` script in, a wave file and a CSV out.

```bash
cargo build --release
./target/release/engine-sim-cli ../assets/main.mr --seconds 6 --throttle 1 \
    --wav /tmp/ej25.wav --csv /tmp/ej25.csv

engine    Subaru EJ25
layout    2 banks, 4 cylinders, 1 exhaust system(s), 1 intake(s)
frames    327 frames, 120009 simulation steps (6.00 s of engine time at 60 fps)
rpm       89 to 6855 (final 6640)
dyno      not engaged (--dyno <rpm> holds the engine and fills torque/power)
audio     264620 samples (6.00 s at 44.1 kHz)
  wrote   /tmp/ej25.wav (264620 samples)
  wrote   /tmp/ej25.csv (327 rows)
```

```
engine-sim-cli <script.mr> [--seconds s] [--fps n] [--wav path] [--csv path]
               [--throttle 0..1] [--dyno rpm] [--gear n] [--shift s]
               [--clutch 0..1] [--starter s] [--no-starter] [--no-ignition]
               [--search dir]... [--quiet] [--timing]
```

The wave file is 44.1 kHz, 16-bit, mono — what the synthesizer produces — and the
CSV is one row per application frame: `frame,time_s,rpm,vehicle_kmh,crank_rad_s,
throttle,dyno_torque_nm,dyno_power_w,exhaust_pa,chamber0_pa,audio_samples`.

Four things it does that are worth knowing, because they are the application's
behaviour rather than the engine's:

* **The audio is rendered even without `--wav`.** The synthesizer and combustion
  draw from the same random sequence, so skipping it changes the physics.
* **The starter is released after `--starter` seconds** (default 1). Its
  constraint *drives the crankshaft at the starter's speed*, so holding it — what
  every parity trace does — would sit at 500 rpm forever, however wide the
  throttle is.
* **The ignition is switched on**, which the engine does not do for itself: the
  module is built disabled and the application's `A` key is what enables it. Run
  with `--no-ignition` and the engine compresses and dies, which is what the
  original does too.
* **The car is the script's own**, with the application's 1597 kg / six-speed
  pair only where the script published none — `loadScript` falls back per
  *object*, so a script may bring one and not the other. Every engine under
  `assets/engines/` brings both, and `assets/main.mr` brings the Impreza its
  engine was recorded from (2700 lb, six ratios from 3.636 to 0.756), so
  `--gear`, `--shift` and `vehicle_kmh` are about a real driveline.
* **The clutch is engaged but the gearbox starts in neutral**, which is why
  nothing moves until `--gear` says so: `Transmission` begins at zero clutch
  pressure, and in neutral its limits are zero anyway. `--gear` alone engages at
  frame zero, which stalls the engine on the starter; `--shift 2` is the same
  shift made once the engine is running, and the car pulls away.

`--dyno <rpm>` holds the crankshaft at that speed, the way the application's dyno
does. The dynamometer's torque is the constraint's *reaction*, so the API reports
it negative while the engine drives it; the summary negates it, the CSV keeps the
raw value — and, because it is a reaction, it is also large and *positive* when a
dead engine is being dragged. Held at 3000 rpm, the Subaru EJ25 needs 240 Nm to
turn with the throttle shut and supplies 30 Nm (39 kW at the peak of the
transient) when it fires wide open. Read the trace, not one number: the free
engine, wide open from the starter, runs up to 6855 rpm — its redline — in six
seconds.

## Parity harnesses (the important part)

Both harnesses compile the repo's **own** C++ sources with `g++`, run the same
scenario as a Rust binary, and diff raw `f64` bit patterns:

```bash
./tools/parity/run_parity.sh                  # gas system only, 185 checkpoints
./tools/parity/run_core_parity.sh 200000      # whole core, 24 fields per step
PARITY_DYNO=1 ./tools/parity/run_core_parity.sh 20000   # same, dyno hold engaged
./tools/parity/run_dsp_parity.sh              # every audio filter + the synthesizer
./tools/parity/run_script_parity.sh           # .mr front end: IR, RUN, DRIVEN, RUNNING, GRAPH, VALUES, OBJECTS, OPTIMIZE, EXECUTE
./tools/parity/measure_performance.sh         # not a gate: how fast either side renders (real-time factor)
RECORD_RESOLVED=1 ./tools/parity/run_script_parity.sh   # re-record the M2 targets
RESOLVED_GAP=1 ./tools/parity/run_script_parity.sh      # how much of M2 is still open
RECORD_ERRORS=1 ./tools/parity/run_script_parity.sh     # re-record the error fixtures

./tools/parity/stage_reference.sh             # first run only: builds /tmp/refcore
./tools/parity/stage_script_reference.sh      # first run only: builds /tmp/refscript
```

`stage_reference.sh` does the reference build: it initialises the two C++
submodules (`simple-2d-constraint-solver`, `csv-io`), copies `include/` and
`src/` to `$REF_CORE` (default `/tmp/refcore`), applies `reference.patch` (the
five mechanical edits below), compiles every core translation unit except the
Win32/D3D app shell, and archives the result as `librefcore.a`. `run_core_parity.sh`
calls it automatically when the archive is missing.

`run_core_parity.sh` builds `tools/parity/core_reference.cpp` (a complete
`PistonEngineSimulator` configured exactly like `examples/core_trace.rs`) and
emits `<step>,<field>,<16 hex digits of the f64>` for every traced quantity:
crank angle/velocity, piston and rod pose, chamber pressure/temperature/moles/
mixture/volume/lit state/AFR/exhaust flow, intake/runner/exhaust pressures, the
ignition advance and the dyno torque.

`reference.patch` contains everything needed to compile the upstream sources
outside MSVC — none of it changes arithmetic:

1. `const Mix &mix = {}` default arguments become `= Mix()` (a `Mix()` default
   constructor is added to `struct Mix`); GCC/Clang reject the brace form.
2. Windows path separators in three `#include`s become forward slashes.
3. `fpclassify` becomes `std::fpclassify` in `synthesizer.cpp`.
4. The destructor assert on `m_antialiasingFilters` is removed — that member
   **does not exist anywhere in the repository**, so the shipped
   `piston_engine_simulator.cpp` cannot compile with any compiler (see findings).
5. `include/delta.h` is stubbed out (it only pulls in `delta-studio`).

Two compiler-level concessions are needed as well (again, no arithmetic
changes):

* `extern constexpr double` in `units.h` / `constants.h` defines external
  symbols, so they are multiply-defined across translation units;
  `-Wl,--allow-multiple-definition` lets the linker accept identical constants.
* The headers lean on MSVC behaviours (`__forceinline`, `-fpermissive` for
  `class Fuel` vs `Parameters::Fuel`, members used before their declarations),
  so the reference build passes `-fpermissive -D__forceinline=inline
  -include cmath -include cstdlib -include cstring`.

`run_dsp_parity.sh` is the same idea one level down: it drives every filter of
the audio chain (and then a whole `Synthesizer`) with an identical,
libm-independent input on both sides and compares the raw `f64` bits of every
output. It exists because a 1-ulp coefficient difference is invisible at the
engine level — it only shows up as a handful of `int16` samples that are off by
one, thousands of samples into a run.

### Making combustion reproducible

`CombustionChamber::ignite` draws one `rand()` per ignition event, so the
original is not reproducible across C libraries. For parity both sides use the
same 31-bit LCG (`fuel::Rng` in Rust, `tools/parity/parity_rand.{h,cpp}` on the
C++ side): the reference build recompiles each `rand()`-using translation unit
with `-include parity_rand.h`, which force-defines `rand()` after `<cstdlib>`
(libstdc++ `#undef`s it) and links those objects ahead of `librefcore.a`.

There are two such units: `combustion_chamber.cpp` (ignition efficiency) and
`synthesizer.cpp` (`Synthesizer::renderAudio` draws two values per sample for
the jitter term). Both harnesses recompile both. Missing the synthesizer one is
subtle: the engine trace stays bit-exact and only the audio differs, because the
`int16` output saturates the difference into a handful of 1-LSB samples.

## Findings worth knowing (found while porting)

* **`src/piston_engine_simulator.cpp` does not compile at all as committed.**
  Its destructor asserts `m_antialiasingFilters == nullptr`, but that member is
  not declared in any header (nor defined anywhere else in the repo) — the
  line is a leftover from a removed antialiasing stage. This is worth knowing
  before trusting "it builds on Windows": whatever binary the author shipped
  was not built from this tree, or was built after a local edit.
* **The C++ tree is MSVC-only in practice.** `include/gas_system.h` does not
  even parse with GCC 12/Clang in any `-std=` mode, because of the
  `const Mix &mix = {}` default arguments. Any claim that the current codebase
  "just needs a platform layer" is wrong in the details; the headers themselves
  need touching.
* **Three upstream gtest cases already fail on the original C++.** They are
  pinned here as *characterisation* tests
  (`pressure_equilibrium_max_flow*_is_characterised`) with the values produced
  by the C++ build. `pressureEquilibriumMaxFlow` returns a signed flow measured
  out of `self`, and the upstream tests feed that negative number into `gainN`,
  which drains the volume (one case ends at `P = 0`, another at a negative
  pressure). Those tests look like they predate a sign-convention change and
  are not run by CI (the workflow only builds).

### Two DSP bugs the harness caught

1. **Butterworth coefficients were 1 ulp off.** `setCutoffFrequency` mixes
   precisions: `f`, `f_2`..`f_4`, `(m + n) * f` and `m * n` are all *float*
   arithmetic, while the `2.0` literals promote `(2.0 + m * n)` and the whole
   polynomial sum to `double`. Doing the `m * n` product in double (the obvious
   reading of "it is mixed precision, so use the wider type") changes several
   coefficients in the last bit at 10 kHz — the air-noise cutoff. Everything
   still sounds fine; the noise floor just moves by an ulp and a few audio
   samples end up 1 LSB apart.
2. **`std::tan` is not portable across time.** Rust constant-folds `f32::tan`
   when LLVM can see a constant cutoff, and the folded value differs from the
   `tanf` the reference calls at run time. The unit test that pins the
   coefficients therefore passes its cutoff through `std::hint::black_box` to
   force the same runtime call the C++ makes.

### Four core bugs the parity harness caught (all in ported code, now fixed)

Each of these produced plausible-looking but wrong engine output — audio would
have been subtly (or grossly) wrong with no test noticing:

1. **Constraint registration order.** `Engine::buildEngine` registers
   `little_end`, `big_end`, then `wall`; the port had `wall` first. Gauss-Seidel
   sweeps constraints in registration order, so the solution differs in the
   last bits of the piston's x coordinate, which then grows into a completely
   different engine (the trace diverged by 3e-10 at step 0 and never
   re-converged).
2. **Dyno sample buffer wrap-around.** `PistonEngineSimulator` fills its
   512-sample torque ring buffer with a `for (i = last + step; i != index; i +=
   step)` loop whose `continue` still applies the increment. The `while` port
   skipped the increment on wrap and spun forever the first time the index
   wrapped backwards (~step 2 500) — a hang, not a wrong number.
3. **Ignition advance sign.** The reference samples the timing curve at
   `-v_theta` (signed), not `-|v_theta|`. The port used the magnitude, so a
   clockwise crank sampled the curve at negative speed, clamped to the first
   sample (12° instead of ~13.6°), and the first combustion event landed one
   step late. Everything up to that point was bit-exact, which is exactly why
   the divergence looked like a gas/combustion bug.
4. **Combustion is random.** See above: without pinning `rand()`, combustion
   cannot be compared at all, and it is not reproducible between two runs of
   the *original* either.

### `setSpeedControl` is inverted from the plate, and both drivers had it backwards

`DirectThrottleLinkage::update` turns the speed control into a plate position with
`position = 1 - control^gamma`, and `Intake` measures its flow with
`cos(position * pi/2)` — so **a control of one is wide open and a position of zero
is wide open**, and the application's `Q`/`W`/`E`/`R` keys (0.01, 0.1, 0.2, 1.0)
are an opening throttle in that order. Both `.mr` drivers assumed the opposite
and passed `1 - throttle` to `setSpeedControl`, which made every "wide-open" run
a nearly shut one: the EJ25 "revved" to 790 rpm, chamber peaks were 16 bar, and a
dyno pulled it round at 240 Nm. Nothing caught it because the port and the oracle
were wrong in the same way — the gate compares them to each other. It surfaced
only when the drivetrain milestone asked the engine to move a car.

With the sense corrected, wide open from the starter takes the EJ25 to 6855 rpm,
and the `DRIVEN` gate now runs an engine that is genuinely at wide-open throttle
rather than one that is coasting.

### Every `.mr` gate was cranking, never running

`IgnitionModule` is constructed with `m_enabled = false` and nothing enables it
except the application's `A` key. Both `.mr` harnesses inherited that, so for
every trace in `run_script_parity.sh` the engine turned over and never fired —
400 frames of a starter motor, bit-exact in every field, and blind to the whole
combustion path of the script-built engine. It only mattered when the CLI tried
to run an engine for real: it sat at the starter's speed and then stopped,
because no cylinder had ever lit. The fix is one line in the driver, and the
`DRIVEN` gate is the test that would have caught it.

### The dyno hold stalls this fixture (in the original, too)

Holding the dynamometer at a fixed speed keeps the TRX520 fixture at a few
hundred rpm with ~1.2 atm of cylinder pressure and no combustion. That is
reference behaviour, not a port bug: `PARITY_DYNO=1` reproduces it
bit-for-bit. The end-to-end combustion test therefore runs the free-revving
starter configuration, where the first charge lights at ~0.9 s.

## The audio path

The engine's "sound" is a single `f64` per exhaust per frame
(`Simulator::writeToSynthesizer`), resampled from 10 kHz to 44.1 kHz and run
through a fixed chain: jitter (with its own `std::default_random_engine` and two
`rand()` draws per sample in `renderAudio`), a DC-blocking low pass, a
derivative/level mix, a 512-tap convolution with the impulse response the script
names (or a synthesised one when it names none),
a 4th-order Butterworth, a leveling filter, and an `int16` round-and-clamp.
The port reproduces all of it operation for operation, including:

* `RingBuffer`'s latent bugs (`overwrite`'s undeclared `start`, the
  single-capacity subtraction in `removeBeginning`/`readAndRemove`);
* `ButterworthLowPassFilter`'s 4-slot shift register — it is *not* a biquad, and
  writing it as one changes the output;
* `JitterFilter`'s history layout, `floor`/`ceil` interpolation and clamping,
  and libstdc++'s `generate_canonical<float, 24>` for
  `uniform_real_distribution<float>`;
* the reference's own instability: at 100 Hz / 44.1 kHz the direct-form f32
  Butterworth overflows to infinity after ~29 000 samples. The synth only uses
  1900 Hz, 10 kHz and 0.45·fs, all stable; the divergence is pinned as a
  characterisation test rather than "fixed".

Rendering is driven explicitly (`Simulator::render_audio_block`) instead of on
the reference's audio thread, so PCM output is reproducible.

## Real time

`tools/parity/measure_performance.sh` runs every engine asset for two seconds of
audio on both sides and reports the real-time factor — audio seconds per second
of wall clock. It times the *loop*, not the process: the port reports its own
(`engine-sim-cli --timing`), the original reports its own (`script_reference
--bench`), and the shell times the setup around them, because on the
nine-cylinder radial a 4 s script-to-engine build is twice the run it measures.

| | port | original |
|---|---|---|
| all 21 scripts, 2 s each | **1.30x real time** | 1.13x |
| slowest (the nine-cylinder radial) | 1.02x | 0.89x |

Rendering is 44 100 samples a second through a 10 000-tap direct convolution per
channel — the original's algorithm, and its order of summation is what makes it
bit-exact, so an FFT or a partitioned convolution is not available. What *is*
available is overlap: `ConvolutionFilter::f_pair` runs the two channels' chains in
one pass (bit-identical by construction, 1.6x the multiply-adds per second and
19-22% off the loop on the two-exhaust engines), and `write_to_synthesizer` no
longer clones a cylinder head per cylinder per step. `docs/mr-interpreter-plan.md`
§8.7 has the measurements, and the wrong instrument that first reported the port
as slower than it is.

## Where the port stands

The `.mr` pipeline is complete — front end, resolution, validation, node graph and
execution, the `scripting/` bindings (engine, vehicle and transmission), the wave
impulse responses, and a command line that runs a script without the application.
What is left is the app shell.

* **M2 — resolution and validation.** Types, default values, visibility,
  aliasing, `inline`, `=>` shadowing — gated by
  `fixtures/{es_engine_sim,assets_main}.resolved` (660/660 and 701/701 rows). The
  substrate is `src/lower.rs` (parse tree → identity-preserving arena, because
  piranha resolves in place and compares structures by identity; see
  `docs/mr-interpreter-plan.md` §8.3), with `src/channel_type.rs`,
  `src/error.rs` and `src/context.rs` (`IrContextTree`).
* **M3 — node graph and execution** (`Node`, `NodeOutput`, `Channel`,
  `NodeProgram`, allocator, the mutation log). The original executes nodes in
  `NodeContainer` order and never calls `NodeProgram::optimize()` — the
  application does not — so the order is insertion order, and every `_evaluate`
  side effect matches. Gated by `GRAPH`, `VALUES`, `EXECUTE` (four fixtures, one
  of which reads standard input) and `OPTIMIZE`. The runtime's four remaining
  piranha features are closed rather than pending, and each is closed differently:
  `console_in` is ported and gated (`Graph::outputs` carries the one value a run
  writes into its own output), `checkEnabled` is dead (`connectEnableInput` has no
  caller anywhere, so every node is enabled), `optimize`/`markDeadNodes` are
  measured to change nothing (28 runs identical with and without, including the
  whole engine trace) and `free` is a drop.
* **M4 — the `scripting/` bindings**: `EngineContext`, `EngineNode::buildEngine`,
  every `*_node.h`, and the `actions.h` entry points. Gated by `script_reference
  --run`/`--engine` against the recorded
  `crates/engine-sim-script/fixtures/main_engine.trace`, by `DRIVEN` for a
  running engine, and by `DRIVE` for the drivetrain — the vehicle and the
  transmission the script publishes, built by `build::simulator` on the same
  per-object fallback rule as `EngineSimApplication::loadScript`. That was the
  last gap in M4 (`docs/mr-interpreter-plan.md` §9).
* **M5 — the `.wav` impulse responses** in `es/sound-library/`. `src/wav.rs` is
  `ysWindowsAudioWaveFile`'s parse (chunks, block alignment, `dataSize /
  blockAlign` frames) and `src/paths.rs` is the filename rule
  `ImpulseResponseNode::generate` applies; `build` returns the responses beside
  the engine and `simulator` installs them, so the trace's audio is a
  convolution over the real samples. Gated by `WAVE`.
* **M6 — the command line**, `crates/engine-sim-cli`: a script in, a wave file and
  a CSV out, with the application's own keys as flags.
* **App shell** (window, render, UI, audio output) last; it is where the port can
  finally diverge from the C++ without losing fidelity.

## The `.mr` IR substrate

`lower.rs` turns the parse tree into an arena of `StructureId`s. piranha resolves
in place and keys several algorithms on pointer identity, so a tree cannot carry
it; the arena keeps `IrParserStructure`'s fields (parent, scope parent, logical
parent, summary token, visibility, stage flags) and reproduces
`registerComponent` order exactly. `Arena::to_unit` raises it back, and
`tests/lower_tests.rs` asserts `to_unit(lower(unit)) == unit` for all 18
standard-library and all 21 asset units — 253/653/641 and 409/1026/677
nodes/attributes/ports respectively.

`fixtures/{es_engine_sim,assets_main}.resolved` are the **M2 target**, recorded
from the original: one `P,` row per node-definition port (unit, definition, port,
direction, alias, the definition the declared type resolved to, and the channel
type `getImmediateChannelType()` produced) and one `A,` row per unit-level
attribute (the port it resolved to). 660 and 701 rows. Nothing compares against
them yet — that is M2c — but they record what `resolveDefinitions` +
`checkTypes` actually decided, which is otherwise invisible.
`examples/resolved_dump.rs` emits the same rows from the lowered IR, and
`tests/resolved_tests.rs` requires them to be **identical** to the fixture:
660/660 for `es/engine_sim.mr`, 701/701 for `assets/main.mr`. That is the M2
resolution gate — definition lookup (imports, visibility, `ns::name`, `::name`),
port types, the alias-type hop (`[::string]` resolves to `string` whose alias
output is `string_channel`), attribute→port matching (positional and named), and
the builtin channel-type table. `RESOLVED_GAP=1` still prints the scored diff for
debugging.

Validation has its own gate, and it needs scripts that are *wrong*:
`tools/parity/negative/*.mr` are twelve broken scripts whose exact error-code
multiset was recorded from `script_reference --errors` (V0030 duplicate symbols,
V0060 duplicate definitions, V0010/V0011 input given twice — the positional form
reports the `...Positional` code — V0040 input not connected, V0050 output with
no definition, R0010/R0020/R0030/R0040 resolution errors).
`tests/error_tests.rs` requires the Rust front end to reproduce them all.

`channel_type.rs` is `piranha::ChannelType` (name, parent chain, byte-sum hash,
`isCompatibleWith`) plus the registry that interns types; `error.rs` is the
whole `ErrorCode` table (`compilation_error.cpp`) with the exact stage/code/info
strings, since those land in `error_log.log`.

## The `.mr` front end

`crates/engine-sim-script` is the port of piranha's scanner, parser and IR. It
is dependency-free like the core crate and is verified against the original
rather than against its own expectations.

Three things made that possible:

* **`scanner.l` is 170 lines and `specification.y` is 525.** Both were ported
  rule for rule (`parser.rs` mirrors the grammar's precedence ladder
  `value > add_exp > mul_exp > unary_exp > data_access > primary_exp >
  atomic_value`). flex's longest-match behaviour matters: `importx` is a label,
  `1.5` is one float token, `0x1F` is one int.
* **Only the parser needs bison.** `stage_script_reference.sh` builds piranha
  and the whole `scripting/` layer from the repo's own sources with the parser
  removed, so the oracle covers everything downstream of the grammar.
* **The IR is dumped as S-expressions** (`dump.rs`) and read back into real
  piranha objects by `script_reference --ir`. `compare_ir.py` then compares the
  two trees, so a mismatch points at the exact field
  (`/unit[0]/def[3]/ports[2]/default/...`) that differs.

The gate (`run_script_parity.sh`) currently reports:

```
-- 80 files round-tripped and validated, 0 mismatches
PASS: 16265 engine/trace checkpoints match the recorded fixture
PASS: 808301 checkpoints of a running engine are identical
PASS: 764159 checkpoints, .../engines_atg-video-1_01_honda_trx520.mr
PASS: 808301 checkpoints, .../vehicle_without_transmission.mr
PASS: 808301 checkpoints, .../drivetrain_from_application.mr
```

The second line is the interesting one: the oracle resolves, validates, builds
and *executes* `assets/main.mr` from the Rust parser's IR, and the resulting
engine is the Subaru EJ25 — 2 banks, 4 cylinders, 2.457 L displacement, 79 mm
stroke (`crank0_throw = 0.0395`), 130.6 mm rods, 67 cc chambers, 19.8 cm² intake
runners — exactly what the script declares. That output is now the target for
M4.

The third line is the newest one, and it exists because the first two are both
*cranking*: 400 frames with the starter held and the ignition off, which the
application does before `S` and `A` are pressed. Nothing in them fires, so
nothing in them could catch a difference in combustion or in the spark's timing.
`DRIVEN` presses the keys — ignition on, starter held for a second and released,
throttle wide open — and runs **20 000 frames instead of 400**: two seconds of an
engine that is running. All 808 301 checkpoints match. The harness takes the same
`PARITY_*` environment variables as the port's example (`PARITY_THROTTLE`,
`PARITY_STARTER_SECONDS`, `PARITY_IGNITION`, `PARITY_DYNO_RPM`, `PARITY_CLUTCH`,
`PARITY_GEAR`), so any driven scenario can be compared by hand:

```bash
PARITY_STEPS=20000 PARITY_THROTTLE=1 PARITY_STARTER_SECONDS=1 PARITY_IGNITION=1 \
    ./tools/parity/run_script_parity.sh       # (the gate does exactly this)
```

`DRIVE` is the third layer and the newest: the drivetrain. `assets/main.mr`
publishes a car and a gearbox, so from M4 onwards the gates were driving whatever
`loadScript` chose — and every one of them drove it in **neutral**, where a
transmission's ratios do nothing and a vehicle's mass only sets a drag. `DRIVE`
puts three fixtures under `tools/parity/engines/` in first gear with the clutch
out: one that publishes both (the TRX520's 500 kg and five ratios), one that
publishes only a car (a 900 kg body in front of the application's gearbox), and
one that publishes neither (the EJ25's engine with the application's 1597 kg /
six-speed pair). Each combination of `loadScript`'s two fallbacks is therefore
compared rather than assumed, and it is the only place the gear ratios are
compared at all: `Transmission` and `TransmissionNode` keep theirs protected, so
neither dumper can print them — a wrong ratio moves the engine instead. The three
runs are 20 000 frames each and all of them are bit-identical.

Four MSVC-isms had to be patched in the staged copy of piranha/scripting (never
in the submodule), none of which change arithmetic:
`static_assert(false, ...)` inside a template, `template <> extern inline`,
in-class explicit specialisations of member templates, and a member template
parameter that shadows its class template's. `unsigned __int64` and
`boost::filesystem` (→ `std::filesystem`) are in the same category.
