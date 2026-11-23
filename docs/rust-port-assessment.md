# Converting engine-sim to native Rust — effort & risk assessment

Reviewed commit: `85f7c3b` (repo snapshot dated 2023‑01‑22, MIT licensed throughout).
Scope reviewed: this repo (`src/`, `include/`, `scripting/`) plus its five submodules
(`delta-studio`, `piranha`, `simple-2d-constraint-solver`, `csv-io`, `direct-to-video`),
which were cloned at their current upstream heads for measurement.

---

## 1. TL;DR

| Goal | Effort (person‑months) | Calendar (1 senior dev, FT) | Likelihood of finishing |
|---|---|---|---|
| Rust **simulation + audio core** (library/CLI, no GUI) | 6 – 12 (realistic 8) | 8 – 14 months | **85 %** |
| Rust **native app**, own config format, no `.mr` compatibility | 12 – 24 (realistic 17) | 18 – 28 months | **60 – 70 %** |
| Rust **native app** that still runs existing `.mr` engine scripts | 15 – 30 (realistic 21) | 22 – 34 months | **45 – 55 %** |
| **Drop‑in parity** replacement (scripting, visuals, video capture, community content) | 24 – 40 | 3 – 4+ years | **≈ 30 %** |

The distribution of difficulty is very lopsided: **the actually interesting part (engine
physics + DSP ≈ 8 k LOC) is a high‑confidence port. The parts that make it an *application*
(the 35 k LOC Windows‑only engine, and the 14 k LOC scripting language) are rewrites, and
they are where the schedule goes.**

My recommendation: **port the core, rewrite the shell, and keep `.mr` compatibility only if
the community content is the point** (see §6).

---

## 2. Code inventory (measured)

### This repository

| Part | Files | LOC | Notes |
|---|---|---|---|
| Simulation core (`engine-sim` static lib) | 42 `.cpp` + headers | **6,711** | gas dynamics, combustion, valvetrain, crankshaft, dynamometer, vehicle, transmission |
| DSP / audio | (inside above + headers) | ~1,500 | synthesizer, ~11 filters, convolution, ring buffers |
| App + UI (`engine-sim-app`) | 31 `.cpp` + headers | **8,135** | render loop, procedural geometry, ~12 gauge/plot widgets, engine view |
| Script interpreter (`es_script`) | 4 `.cpp` + 30 headers | **3,121** | node classes that build engines from scripts |
| Tests | 3 files | 1,083 | gas system (813), synthesizer (190), function (80) |
| Engine/theme scripts | 58 files | 11,121 | 27 engine definitions, part library, 6 theme scripts |

### Submodules (all MIT, all first‑party C++)

| Dependency | LOC | Role | Port strategy |
|---|---|---|---|
| `delta-studio` | **35,431** | 2D/3D engine: Win32 window, D3D10/11 + WGL/GL + partial Vulkan, DirectSound‑8 audio, text renderer, asset manager, UI primitives, physics | **Do not port** — replace with `winit` + `wgpu` + `cpal` + `rustfft` + a 2D renderer |
| `piranha` | **10,469** (+695 flex/bison) | the language behind `.mr`: lexer/parser, IR, type system ("channel types"), node graph, VM | **Replace** with a Rust config format, or FFI‑embed piranha |
| `simple-2d-constraint-solver` | **5,144** | 2D rigid‑body solver used for the crank/rod/piston chain | Port the subset actually used (~2 k LOC) |
| `csv-io` | ~700 | CSV export used by tests/app | `csv` crate, trivial |
| `direct-to-video` | ~800 | FFmpeg‑based video capture (build option) | `ffmpeg-next`, trivial |

**Total C++ in scope ≈ 70 k LOC**, of which ~18 k is this repo's own code.

---

## 3. Why this is more tractable than 70 k LOC sounds

* **Clean, old‑fashioned C++ subset.** No `dynamic_cast`, no `typeid`, no smart pointers,
  no SIMD intrinsics, 2 `throw`s, 4 uses of `template`, no multiple inheritance, one
  `std::thread`, no networking, no async, no dependency injection. 255 virtuals and raw
  `new[]`/`delete[]` arrays — which map to `Box`, `Vec`, and enums/dyn traits.
* **No exotic maths libraries.** `exp`, `sin`, `sqrt`, `hypot`; no linear‑algebra stack,
  no FFT library (convolution is a direct loop). Everything is `f64`/`f32` scalar code.
* **Deterministic shape.** Fixed‑timestep integration (10 kHz nominal) driven by frame time;
  a handful of explicit integration steps per frame. Golden‑trace comparison against the
  C++ build is genuinely feasible, and Rust (no FP contraction by default) is *more*
  reproducible than MSVC `/fp:fast` builds.
* **Existing tests.** 1,083 LOC of gtest covering gas‑system energy conservation, entrainment,
  and the synthesizer gives you a ready‑made conformance harness to translate.
* **Licensing is a non‑issue.** MIT across app + all submodules; assets included in‑repo.

## 4. Why it is still hard (the real risks)

1. **The product is a *feel*, not a spec.** The author calls the code "sloppy"; combustion,
   valve flow, and exhaust acoustics are tuned heuristics. Fidelity therefore requires
   *transliteration* — same operations, same order, no "improvements". Any refactor that
   changes summation order or filter topology changes the sound, which is the whole product.
2. **`piranha` is the compatibility trap.** `.mr` is the engine‑definition format and the
   community ecosystem is hundreds of these files. Reimplementing a flex/bison‑based
   language with an IR, custom type system and node graph is 4–9 PM of unglamorous work;
   dropping it invalidates every community engine.
2b. **The headers themselves are MSVC‑only.** While starting the port (see §10) it
   turned out `include/gas_system.h` does not compile with GCC or Clang in any
   `-std=` mode: `const Mix &mix = {}` default arguments inside a class
   definition are an MSVC extension. So even the "pure" simulation core needs
   small header edits before it can be built on Linux — another reason a
   translator layer over the existing C++ is not the cheap option it looks like.
3. **`delta-studio` is not portable and not really "portable‑isable".** Window + input +
   audio are Win32/DirectSound‑8 only, and graphics paths are D3D10/11, WGL‑OpenGL, and a
   partial Vulkan. There is no Linux/macOS build today — **which also means you cannot build
   the reference *application* on your dev box to capture golden output; you need a Windows
   VM first.** That is a hard prerequisite for a faithful port, not a nice‑to‑have.
   (Correction after §10: the *simulation core* alone does build on Linux once the two
   MSVC‑isms in §4.2b are patched, which is enough to generate golden traces headlessly —
   see `rust/tools/parity/run_core_parity.sh`.)
4. **Asset pipeline.** `assets/assets.ysce` (530 KB) is a proprietary binary mesh/scene
   format produced by a Delta Studio Excel/Blender toolchain (`assets.dia`, `art/assets.blend`).
   Either reverse‑engineer the loader or re‑export ~8 models (`Crankshaft`, `Piston`,
   `ConnectingRod`, `CylinderHead`, `Valve`, …) to glTF — the latter is cheaper and is what I
   would do.
5. **Closed audio loop.** The synthesizer's measured latency feeds back into how many sim
   steps run per frame (`Simulator::startFrame`), and the sim both consumes and produces
   audio. Port the loop *and* the scheduling, or the engine idle/rev behaviour changes.
6. **Big custom UI surface.** Not a widget‑toolkit app: a procedural 2D vector renderer,
   ~12 bespoke widgets (gauges, oscilloscope, AFM/AFR clusters, firing‑order display, mixer),
   custom shaders, text atlas, themes. This is 8 k LOC of the most bespoke, least mechanical
   code in the repo — hence 4–14 PM depending on how much visual parity you demand.
7. **Snapshot drift.** Upstream development moved to the community edition; this repo is a
   2023 demo. Port the *right* baseline, or you inherit known bugs.

---

## 5. Per‑subsystem estimate

Person‑months for one senior developer fluent in both C++ and Rust (~5–6 productive h/day).
"Realistic" assumes the port is test‑driven against C++ reference traces.

| Subsystem | LOC | Strategy | Opt. | Real. | Pess. |
|---|---|---|---|---|---|
| `GasSystem` + combustion chamber + fuel/AFR | ~1,700 | transliterate + conservation tests | 1.0 | 1.5 | 2.0 |
| Engine graph: crankshaft, banks, heads, valvetrain (std + VTEC), pistons, rods, intakes, exhaust | ~2,800 | transliterate | 1.5 | 2.0 | 2.5 |
| Dynamometer, starter, governor, transmission, vehicle, throttle linkages | ~1,200 | transliterate | 0.5 | 1.0 | 1.5 |
| `atg_scs` subset (NSV optimized system, Gauss–Seidel SLE, 8 constraint types) | ~2,000 of 5,144 | reimplement | 1.5 | 2.0 | 3.0 |
| Synthesizer + 11 filters + convolution + real‑time thread + `cpal` out | ~1,500 | transliterate | 1.5 | 2.0 | 3.0 |
| `utilities`, `units`, `Function`, settings, CSV export | ~600 | port | 0.25 | 0.5 | 1.0 |
| **Subtotal: headless, native Rust "engine + sound" library/CLI** | | | **6.3** | **9.0** | **13.0** |
| Native shell: window, input, GPU 2D renderer, text, geometry generator | (rewrite) | `winit`+`wgpu`+`glyphon`‑style | 3.0 | 5.0 | 8.0 |
| UI screens & widgets (gauges, scopes, clusters, engine view, themes) | 8,135 total app LOC | rewrite | 3.0 | 6.0 | 10.0 |
| Model/asset import (glTF re‑export of 8 `.ysce` models) | — | `gltf`/`meshopt` | 0.5 | 1.0 | 2.0 |
| Engine definition format (`.mr` replacement) + convert 27 engines, part library, 6 themes | 11,121 script LOC | serde + TOML/RON + conversion | 1.0 | 1.75 | 3.0 |
| *Alternative:* reimplement `piranha` (parser/IR/VM/node graph) for `.mr` compatibility | 14 k | new compiler | 4.0 | 6.0 | 9.0 |
| Platform odds & ends: Discord RPC, video capture, settings, packaging, CI, 3 OS targets | ~500 | crates | 1.0 | 2.0 | 3.0 |

**Totals**

| Deliverable | Realistic PM |
|---|---|
| Headless core + audio (no UI) | **9** (6–13) |
| Native app, own engine format | **17** (12–24) |
| Native app with `.mr` compatibility | **21** (15–30) |
| Drop‑in parity + visual polish + community content | **27** (24–40) |

Calendar, at ~0.7–0.85 PM per calendar month for a solo full‑timer: 9 PM ≈ 11–13 months;
17 PM ≈ 20–26 months; 27 PM ≈ 32–40 months. Two developers cut calendar time ~40 % (17 PM →
10–12 months) at ~20–30 % PM overhead for coordination; four developers give diminishing
returns on the solver/DSP (hard to parallelise) and mostly help the UI and scripting tracks.

External calibration: `engine-sim-rs` (a real, working Rust re‑implementation) is **7.7 k LOC**
and reproduces the audio chain + *simplified* physics — TOML presets, no constraint solver, no
3D view, TUI only. The iOS port is reported as ~40 k lines of Swift on top of a retained C++
simulator. Both are consistent with the numbers above.

---

## 6. Three strategies, priced

**A. Faithful transliteration of everything (including `piranha` and visual parity).**
Largest artefact, preserves the ecosystem exactly, ~24–40 PM, ~30 % chance of completion as a
volunteer effort. Only worth it if `.mr` compatibility and pixel parity are hard requirements.

**B. Port the core, rewrite the shell (recommended).** `winit`/`wgpu`/`cpal`, native engine
format, keep the DSP/audio architecture and the visual layout but not the exact rendering code.
12–24 PM to parity, viable MVP (headless + simple UI, loads a converted engine) in ~9–11 PM.
Also unlocks Linux/macOS/WASM — the payoff that the current Win32/D3D11 code cannot deliver.

**C. Hybrid / incremental.** Rust core crate (`engine`, `gas`, `dsp`, `solver`) exposed via
`cxx`, driven first by the existing C++ app, then by a Rust app. 5–8 PM to first sound, 85 %
confidence, and it de‑risks everything except the UI. If you can't commit a year, do C then B.

## 7. Suggestions that materially cut the estimate

* **Capture goldens first.** Build the Windows binary in a VM, record: (a) gas‑system states
  at N steps for fixed inputs, (b) synthesized PCM for a fixed engine + throttle + RPM script,
  (c) gauge/telemetry traces. Then every Rust module is diff‑tested rather than eyeballed.
* **Use `libm`** for transcendentals if you want near bit‑exact parity (`std::sin/exp/pow`
  may differ in the last ulp between MSVC CRT and Rust's libm).
* **Keep operation order** and avoid `-ffast-math`/`mul_add` in the DSP path; that alone gets
  you to sample‑identical or ±1 LSB output for most blocks.
* **Embed `piranha` via `cxx` initially** (2–3 PM) rather than rewriting it; replace it later
  if and when the Rust node graph exists.
* **Re‑export `.ysce` models to glTF instead of writing a `.ysce` loader.**
* **Write an automated `.mr` → native‑format converter** even if you also support `.mr`; it
  gives you 27 regression engines for free and a migration path.

## 8. Milestone plan (strategy B)

| # | Milestone | Exit criteria | PM |
|---|---|---|---|
| M0 | Baseline harness | Windows VM builds C++; golden traces + PCM recorded | 0.5 |
| M1 | `gas`, `fuel`, `combustion` | conservation & entrainment tests pass within 1e‑4 | 1.5 |
| M2 | Engine graph + valvetrain + dyno | same torque/pressure curves as C++ on 3 engines | 2.5 |
| M3 | `atg_scs` subset | crank/rod/piston kinematics match to 1e‑9 over 10 s sim | 2.0 |
| M4 | DSP + audio out | audible parity on 3 engines, no dropouts at 44.1 kHz | 2.0 |
| M5 | Headless CLI + engine format | loads converted EJ25/LS/V12, runs, exports WAV/CSV | 1.0 |
| M6 | Native shell | window, input, 2D renderer, text | 5.0 |
| M7 | UI parity | gauges/scope/clusters/engine view/themes | 5.0 |
| M8 | Extras & packaging | Discord RPC, video, 3 platforms, CI, releases | 2.0 |

Note that **75 % of the user‑visible value arrives by M5** (~9–10 PM) — that is the natural
"should we keep going?" checkpoint.

## 9. Bottom line

* Technically this is one of the friendlier ports you could pick: plain C++17, no SIMD, no
  framework‑heavy abstractions, MIT, and a simulation core that is only ~8 k LOC.
* The headline risk is **not** Rust vs C++. It is (a) the `.mr` scripting ecosystem, and
  (b) 35 k LOC of Windows/Direct3D glue that must be rewritten rather than translated.
* If "native Rust" means *cross‑platform, maintainable, same sound*: **~1.5–2 years solo,
  ~10–12 months for two people, 60–70 % success.** If it means *bit‑compatible drop‑in with
  the existing modding ecosystem*: **budget 2.5–3.5 years and treat it as a coin flip.**

## 10. Port status

Milestone **M1 (headless simulation core) is complete and verified
bit-exact**. See [`rust/README.md`](../rust/README.md) for the module table,
commands and findings.

* `rust/` — dependency-free workspace with `engine-sim-core` (~9 000 lines,
  no external crates): `constants`, `units`, `utilities`, `gas`, `function`,
  `parts` (crank, banks, heads, valvetrain, camshaft, pistons, rods, intake,
  exhaust), `fuel`, `chamber`, `drivetrain`, `ignition`, `engine`, `simulator`,
  `dsp` (low pass, Butterworth f32/f64, derivative, delay, convolution,
  leveling, jitter, `<random>` compatibility) and `synth`
  (`Synthesizer`: resampling, impulse response, mixing, `int16` output),
  plus the used subset of `atg_scs` (rigid bodies, link/line/gear/torque
  constraints, NSV ODE solver, Gauss-Seidel SLE solver).
* `rust/tools/parity/run_core_parity.sh` builds the repo's own C++ core against
  the same single-cylinder TRX520 fixture and diffs `f64` bit patterns for 24
  physics quantities *plus* the synthesizer input and every rendered audio
  sample per step: **200 000 steps (20 s of engine time) → 8 964 002
  checkpoints, all identical**, including ignition, flame propagation, gas
  exchange, the solver and the full audio chain. `PARITY_DYNO=1` covers the
  dynamometer-hold path.
* `rust/tools/parity/run_dsp_parity.sh` drives each audio filter and a whole
  `Synthesizer` with identical inputs on both sides and compares every output
  bit — this is what localises a 1-ulp coefficient difference that the engine
  trace only shows as an occasional 1-LSB audio sample.
* `rust/tools/parity/run_parity.sh` keeps the earlier gas-system check (185
  checkpoints, also identical).
* 33 Rust tests (16 unit + 13 gas-system integration + 4 end-to-end).

Findings that changed the plan:

* The headers do not parse outside MSVC (§4.2b) — the reference build needs
  three mechanical patches plus two linker flags.
* **Three upstream gtest cases already fail against the original C++** (signed
  `pressureEquilibriumMaxFlow` fed into `gainN`), so they are pinned as
  characterisation tests rather than "made to pass".
* The original simulation is **not reproducible across C libraries**:
  `CombustionChamber::ignite` consumes the global `rand()`. Parity required
  pinning the RNG on both sides; the port ships its own `fuel::Rng` so a given
  engine always sounds the same.
* Constraint registration order, ignition-advance sign and the dyno ring-buffer
  wrap are all load-bearing; three of them were wrong in the port and only the
  bit-exact harness caught them (§4, risk 1: "transliterate, never rewrite").

Remaining work is the parts that sit *around* the core: the `.mr` script
interpreter and the app shell (window, render, audio device). With the physics
and the audio chain both proven bit-exact, the risk there is ordinary porting
effort rather than model fidelity.

Revised confidence: **headless core + audio chain delivered**; end-to-end
application parity ~85 % (dominated by scripting + platform layer).
