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
| `script` front end (`.mr` scanner, parser, IR) | `piranha/flex-bison/*`, `include/ir_*.h` | ✅ done | IR round trip through the original piranha objects, 58 scripts |
| `script` IR substrate (`lower` arena, `channel_type`, `error`) | `include/ir_*.h`, `channel_type`, `compilation_error` | ✅ done | every unit lowers and raises back field-for-field |
| `script` resolution (definitions, types, attributes) | `piranha/src/ir_*.cpp` | ✅ done | 660/660 + 701/701 rows identical to the reference |
| `script` circularity checks | `piranha/src/ir_*.cpp` | ✅ done | E0020 gated by fixtures |
| `script` validation (reported errors) | `piranha/src/ir_*.cpp` | ✅ done | 15 negative fixtures reproduce the reference's exact code multiset |
| `script` references / expansion / type checks / node graph | `piranha/src/ir_*.cpp`, `node_graph.cpp` | ⬜ | — |
| `scripting/` bindings (`EngineContext` → `Engine`) | `scripting/` | ⬜ | — |
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
cargo test                    # 150 tests (core unit + gas + end-to-end + script + parity gates)
cargo test --release
./tools/parity/run_core_parity.sh 200000   # 8 964 002 bit-identical checkpoints
```

`crates/engine-sim-core/fixtures/trx520.rs` is the shared fixture: it rebuilds
`assets/engines/atg-video-1/01_honda_trx520.mr` with the parameter values the
upstream scripting layer produces. It is `include!`d by the tests and examples
(not a Cargo target).

## Parity harnesses (the important part)

Both harnesses compile the repo's **own** C++ sources with `g++`, run the same
scenario as a Rust binary, and diff raw `f64` bit patterns:

```bash
./tools/parity/run_parity.sh                  # gas system only, 185 checkpoints
./tools/parity/run_core_parity.sh 200000      # whole core, 24 fields per step
PARITY_DYNO=1 ./tools/parity/run_core_parity.sh 20000   # same, dyno hold engaged
./tools/parity/run_dsp_parity.sh              # every audio filter + the synthesizer
./tools/parity/run_script_parity.sh           # .mr front end: IR, RUN, GRAPH, VALUES, OBJECTS, EXECUTE
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
derivative/level mix, a 512-tap convolution with a synthesised impulse response,
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

## Next steps

1. **Script interpreter** for `.mr` (`scripting/` + `piranha`). The front end is
   done (see the next section); what remains is everything that happens *after*
   the parser:

   * **M2 — resolution and validation.** Types, default values, visibility,
     aliasing, `inline`, `=>` shadowing. The oracle already runs these stages on
     the Rust IR (`script_reference --check`), so this milestone can be checked
     stage by stage rather than only at the end. The substrate is done
     (`src/lower.rs`: parse tree → identity-preserving arena, because piranha
     resolves in place and compares structures by identity; see
     `docs/mr-interpreter-plan.md` §8.3), as are `src/channel_type.rs` and
     `src/error.rs`, and `src/name.rs` has the scope walk
     (`resolveName`/`resolveLocalName`, including the rule that a node
     definition resolves locally only and that a unit resolves its *nodes*, not
     its definitions), and `src/context.rs` is `IrContextTree` (frame chains,
     `isEqual`/`isOutside`/`hasParent`/`findContext`/`getMain`,
     `resolveDefinition`). Next: `IrReferenceChain` + `getImmediateReference`
     per structure, then `_checkTypes` and `_validate`.
   * **M3 — node graph and execution** (`Node`, `NodeOutput`, `Channel`,
     `NodeProgram`, allocator, the mutation log). The original executes nodes in
     `NodeContainer` order and never calls `NodeProgram::optimize()` — the
     application does not — so the order is just insertion order, but every
     `_evaluate` side effect has to match.
   * **M4 — the `scripting/` bindings** (~2 500 lines, the largest chunk):
     `EngineContext`, `EngineNode::buildEngine`, every `*_node.h`, and the
     `actions.h` entry points. Gate: `script_reference --run` against the
     recorded `crates/engine-sim-script/fixtures/main_engine.trace`, then the
     same for every engine asset, which retires the hand-written TRX520 fixture.
   * **M5 — the `.wav` impulse responses** in `es/sound-library/`.
2. **App shell** (window, render, UI, audio output) last; it is where the port
   can finally diverge from the C++ without losing fidelity.

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
-- 58 files round-tripped and validated, 0 mismatches
PASS: 16263 engine/trace checkpoints match the recorded fixture
```

The second line is the interesting one: the oracle resolves, validates, builds
and *executes* `assets/main.mr` from the Rust parser's IR, and the resulting
engine is the Subaru EJ25 — 2 banks, 4 cylinders, 2.457 L displacement, 79 mm
stroke (`crank0_throw = 0.0395`), 130.6 mm rods, 67 cc chambers, 19.8 cm² intake
runners — exactly what the script declares. That output is now the target for
M4.

Four MSVC-isms had to be patched in the staged copy of piranha/scripting (never
in the submodule), none of which change arithmetic:
`static_assert(false, ...)` inside a template, `template <> extern inline`,
in-class explicit specialisations of member templates, and a member template
parameter that shadows its class template's. `unsigned __int64` and
`boost::filesystem` (→ `std::filesystem`) are in the same category.
