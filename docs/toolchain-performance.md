# The toolchain, and what it is worth

*Measured 2026-11-23 on the port. The question: for a project that may not change
an output bit, which compiler, which profile, and which flags give the most
performance — and which only look like they do.*

Environment: Intel Xeon at 2.60 GHz (2 cores, 48 KiB L1d, 1.3 MiB L2, 54 MiB L3,
AVX-512F and FMA available). Timings are the CLI's own frame-loop time
(`--timing`), so setup is excluded; builds are **interleaved** across variants and
the best of five or six is reported, because single runs on this box swing ±8%.

Compiler versions here: `rustc 1.100.0` (LLVM 23.1.5) and `rustc 1.88.0`
(LLVM 20.1.5), both official x86_64-unknown-linux-gnu builds.

## 1. Profile and target flags

| build | main.mr (2 s) | radial_9 (1 s) | 412_t2 (1 s) | f136_v8 (1 s) |
|---|---|---|---|---|
| thin LTO (was committed) | 1.850 | 1.412 | 1.173 | 1.370 |
| **fat LTO (now committed)** | **1.756** | **1.167** | **0.989** | 1.137 |
| thin + `target-cpu=native` | 1.695 | 1.176 | 1.039 | **1.125** |
| thin + `target-cpu=x86-64-v3` | — | 1.182 | — | 1.134 |
| fat + `target-cpu=native` | 1.718 | 1.276 | 1.090 | 1.235 |
| fat + `target-cpu=x86-64-v3` | — | 1.198 | — | 1.161 |
| fat + `panic = "abort"` | — | 1.177 | — | 1.142 |

Three findings:

1. **Fat LTO was worth 14%** over thin on the compiler then in use, and is
   portable. `codegen-units = 1` was already set, so this was the last profile
   lever. It landed.
2. **`target-cpu` buys nothing on top.** Native and `x86-64-v3` each match fat LTO
   on their own, but *combining* them with fat LTO is 3–10% slower than either
   alone, reproducibly, in two rounds. Flags do not compose. Neither is portable,
   so neither is a build default.
3. **`panic = "abort"` buys nothing measurable** here.

## 2. The compiler itself

Same source, four builds, one session, interleaved, best of five:

| asset | 1.100 thin | 1.100 **fat** | 1.88 **thin** | 1.88 fat |
|---|---|---|---|---|
| radial_9 | 1.216 | 1.167 | **1.152** | 1.199 |
| ferrari_f136_v8 | 1.173 | 1.137 | **1.120** | 1.165 |
| ferrari_412_t2 | 0.904 | 0.850 | **0.828** | 0.864 |

- **The compiler is worth 1–3%, not 1.4×.** An earlier note claimed the same code
  was 1.4× slower under one compiler than another; that was never measured, and
  the two runs also differed in machine. Measured on one machine, 1.100 and 1.88
  are within a few percent, with **1.88 slightly ahead on all three assets**.
- **The LTO mode reverses between compilers.** Under 1.100, fat beats thin by
  ~4%; under 1.88, thin beats fat by ~3.5%. LTO is not a property you can decide
  once — it has to be re-measured whenever the compiler is pinned or changed.
  This is why `Cargo.toml` records the number it was chosen on, not just the value.

## 3. PGO

Profile-guided optimisation is the biggest remaining lever, and it is measurable:
**4.0–9.5% on top of thin LTO**, on a profile gathered from the whole 21-engine
corpus plus `main.mr`.

| asset | 1.100 fat | 1.88 thin | **1.88 thin + PGO** | vs 1.100 fat |
|---|---|---|---|---|
| radial_9 | 1.169 | 1.158 | **1.067** | −8.7% |
| ferrari_f136_v8 | 1.128 | 1.129 | **1.021** | −9.5% |
| ferrari_412_t2 | 0.848 | 0.831 | **0.772** | −9.0% |
| main.mr | 1.694 | 1.639 | **1.573** | −7.1% |

Two things learned the hard way, both worth writing down before anyone repeats
this:

- **The instrumented build must use the same LTO/codegen-units settings as the
  consuming build.** My first PGO attempt instrumented a fat-LTO build and
  consumed the profile in a thin-LTO one, which inlines differently; it came out
  **1–2% slower than no PGO at all**. Matching the two turned the same workload
  into the 4–9.5% above. A PGO result without that detail is not a result.
- **PGO changes where `tanf` is called from — and that is a gate problem, not a
  product problem.** See §5.

## 4. Windows x64

The product has to run on Linux and Windows x64, so this matters more than the
last percent on Linux.

- **Cross-building works up to the linker.** With `rust-std` and `rust-mingw` for
  `x86_64-pc-windows-gnu`, the whole workspace compiles to Windows object code
  (`engine-sim-core`, `engine-sim-script`, `engine-sim-cli`); it stops at
  `linker x86_64-w64-mingw32-gcc not found`. Nothing in the code objected to
  being compiled for Windows. The `x86_64-pc-windows-gnullvm` target plus
  `lld-link` is the way to finish the link without a MinGW gcc driver.
- **Nothing here can *run* a Windows binary** — no wine, no Windows. So Windows
  correctness has to be owned by CI (a `windows-latest` job with MSVC), and until
  it exists, the Windows build is unverified beyond "it compiles".
- **The two platforms will not produce identical bits, and that is a design
  question, not a bug.** The engine has 81 call sites on `f64`/`f32` methods that
  lower to the platform libm (`sin`, `cos`, `tan`, `exp`, `ln`, `powf`, …). glibc
  and the MSVC/UCRT implementations differ in the last bit for some inputs, so a
  Linux build and a Windows build of the same source will drift apart from the
  first frame. Two acceptable answers:
  1. **Per-platform parity** (the cheap one): both sides of the comparison — the
     port and the original — are built for the same platform, so they agree with
     each other on that platform, and the Linux and Windows artifacts differ.
     This is what the project has today by construction.
  2. **Cross-platform identity** (the expensive one): replace the libm calls with
     a deterministic implementation (the `libm` crate, or vendored polynomials).
     That changes every output bit on Linux too, so the oracle comparison has to
     move with it. Only worth it if a replay or an audio file has to be
     bit-identical across operating systems.
  **Decided: per-platform parity.** Each platform is verified against the original
  built for that platform; the Linux and Windows artifacts may differ from each
  other by the last few bits, and that is documented rather than fought. The
  consequence for CI, which is deliberately not hidden: the Windows job proves
  the port *builds* and passes its own tests where the original lives, while the
  bit-exact oracle comparison runs only on Linux. A Windows *parity* gate (Rust on
  Windows against the oracle on Windows, with the same CRT) is the next step if
  Windows releases ever need the word "exact" attached to them.

## 5. Exactness: what each of these does to the output

| check | lines / files | thin vs fat | 1.100 vs 1.88 | + PGO |
|---|---|---|---|---|
| `core_trace` (gas dynamics, f64 bits) | 89 640 | identical | identical | identical |
| `dsp_trace` (every filter, f64 bits) | 21 375 | identical | identical | **1 286 differ** |
| WAV renders, 3 assets | 3 | byte-identical | byte-identical | **byte-identical** |
| `run_dsp_parity.sh` vs the C++ reference | 21 375 | PASS | PASS | see below |

So PGO moved 1 286 of 21 375 trace lines and **not one audio byte**. The cause is
a genuine trap that the code already documents at `dsp.rs:933`:

> `black_box` keeps the cutoff runtime-valued: when LLVM can see a constant cutoff
> it folds `tan` at compile time, and its folded `tanf` differs from the `tanf`
> the reference calls at run time by an ulp.

The DSP harnesses pass `setCutoffFrequency(10000.0f, …)` as a **literal**, so the
build is entitled to fold `tan` — and PGO changes inlining enough to flip that
decision. The trace then encodes this compiler's folded `tanf` rather than the
run-time `tanf` production actually uses. Both `run_script_parity` and the audio
are unaffected because production cutoffs come from scripts at run time.

Fix, applied in the same commit as this document: make the cutoff opaque on **both**
sides of the comparison (`std::hint::black_box` in `dsp_trace.rs`, `volatile` in
`dsp_reference.cpp`) so the gate measures run-time maths, as production does.
Verified: the gate then passes at 21 375 bit-identical outputs, and the PGO build
produces the same trace as the untuned build. Without the fix, a PGO build — or
any future compiler that inlines differently — fails a gate that says nothing
about the product.

The general lesson: **a gate must not encode a constant-folding decision.** Any
`tan`/`sin`/`exp` whose argument can be constant-folded is a place where a
compiler upgrade can change "output bits" for no product reason.

## 6. How the tools were obtained (and what is still missing)

The usual routes are unreachable from this sandbox — `static.rust-lang.org`,
crates.io, Debian, GitHub release assets and every rustup mirror fail to connect.
Two hosts do work and turned out to be enough:

- **npm** carries whole Rust distributions, repackaged per component and target:
  `@rustbin/rustc-1.88.0-x86_64-unknown-linux-gnu`,
  `@rustbin/rust-std-*`, `@rustbin/cargo-*`, `@rustbin/llvm-tools-*` (which is
  where `llvm-profdata` with zlib came from), plus `rust-std` and `rust-mingw`
  for `x86_64-pc-windows-gnu`/`gnullvm`.
- **The `llvm-runtime*` npm packages** carry LLVM 22.1.8 and 23.1.x `clang`/`lld`/
  `llvm-profdata`/`llvm-objdump` for Linux; note their `llvm-profdata` is built
  *without* zlib.

Still missing, and what each unblocks:

| missing | unblocks | workaround today |
|---|---|---|
| `llvm-profdata` with zlib **for LLVM 23.1** | PGO on 1.100 (PGO is measured on 1.88 only) | `rustup component add llvm-tools` on a normal machine |
| `llvm-bolt` | BOLT on top of PGO (typically a further 2–8%) | none |
| any MinGW gcc driver / `gnullvm` rust-std for 1.100 | finishing a Windows link locally | CI's Windows job |
| wine | *running* a Windows binary locally | CI's Windows job |

## 7. What CI should do about it

This feeds Phase 1 (`docs/next-steps.md` §1) and adds five requirements:

1. **Pin the toolchain in the repository.** Done: `rust/rust-toolchain.toml`
   pins 1.100.0. Without it a developer and CI can silently disagree about the
   compiler — and §2 shows the compiler changes which LTO mode is better.
2. **Build the same way everywhere.** The release profile is the thing being
   benchmarked; no job should override it with extra `RUSTFLAGS` beyond a target.
3. **Never ship `target-cpu=native`.** Artifacts must run where they are sent.
4. **Record what produced a number.** Every figure in these documents should carry
   its rustc version, profile, CPU model and method — as the tables above do.
   A performance history without them is noise with a trend line.
5. **Two jobs for the two platforms, and a PGO job only if it can be reproduced.**
   Android-style matrices are overkill here; `ubuntu-latest` + `windows-latest`
   with the pinned toolchain is enough, and the Windows job is the *only* thing
   that can verify Windows at all (§4). If PGO is adopted, the job needs
   `llvm-tools` for the pinned compiler and must replay the corpus exactly, or the
   profile is not reproducible and the numbers are not comparable.
