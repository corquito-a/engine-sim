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
| DSP / synthesizer | `src/synthesizer.cpp`, filters | ⬜ next | — |
| script interpreter (`.mr`) | `scripting/`, `submodules/piranha` | ⬜ | — |
| app shell (window/render/UI/audio out) | `delta-studio` | ⬜ | — |

**Verification result: the whole simulation core is bit-exact.** Running the
same single-cylinder engine (the `01_honda_trx520.mr` fixture) through the
original C++ and through the Rust port for **200 000 physics steps (20 s of
simulated time, 0.5 s per step)** produces **5 000 000 identical `f64` bit
patterns** — crank/piston/rod kinematics, chamber pressure/temperature/moles
and mixture, intake/runner/exhaust pressures, and dyno torque, at every step.

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

The crate has **no dependencies**, so it builds offline once the toolchain is
present.

```bash
cd rust
cargo build --all-targets     # no warnings
cargo test                    # 5 unit + 13 gas + 4 end-to-end = 22 tests
cargo test --release
./tools/parity/run_core_parity.sh 200000   # 5 000 000 bit-identical checkpoints
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

./tools/parity/stage_reference.sh             # first run only: builds /tmp/refcore
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

### Making combustion reproducible

`CombustionChamber::ignite` draws one `rand()` per ignition event, so the
original is not reproducible across C libraries. For parity both sides use the
same 31-bit LCG (`fuel::Rng` in Rust, `tools/parity/parity_rand.{h,cpp}` on the
C++ side): the reference build recompiles **only** `combustion_chamber.cpp`
with `-include parity_rand.h`, which force-defines `rand()` after `<cstdlib>`
(libstdc++ `#undef`s it) and links that object ahead of `librefcore.a`.

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

### Four bugs the parity harness caught (all in ported code, now fixed)

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

## Next steps

1. **DSP / synthesizer** (`src/synthesizer.cpp`, `src/simple_filter.h`,
   `src/low_pass_filter.cpp`, `delta-studio`'s convolution): the port needs the
   same recorded PCM oracle the gas system used. `Simulator::write_to_synthesizer`
   already produces the per-exhaust flow signal, so the input side is ready.
2. **Script interpreter** for `.mr` (`scripting/`, `piranha`): with the core
   bit-exact, `assets/` becomes a regression corpus — load each engine, run N
   frames, compare traces.
3. **App shell** (window, render, UI, audio output) last; it is where the port
   can finally diverge from the C++ without losing fidelity.
