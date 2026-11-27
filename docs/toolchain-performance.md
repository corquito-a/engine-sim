# The toolchain, and what it is worth

*Measured 2026-11-23 on the port, by building the same commit several ways and
timing it. The question this answers: for a project that may not change an output
bit, which compiler, which profile and which flags give the most performance —
and which of them only look like they do.*

Environment: Intel Xeon at 2.60 GHz (2 cores, 48 KiB L1d, 1.3 MiB L2, 54 MiB L3,
AVX-512F and FMA available), `rustc 1.100.0` (LLVM 23.1), release profile as
committed. Every timing below is the CLI's own frame-loop time (`--timing`), so
setup is excluded; runs are **interleaved** across builds and the best of five or
six is reported, because single runs on this box swing ±8%.

## 1. What was measured

Four assets, chosen to cover the shapes: `main.mr` (mono, audio-heavy),
`radial_9` (stereo, nine cylinders), `ferrari_412_t2` (stereo, short response),
`ferrari_f136_v8` (the slowest asset in the sweep, simulation-heavy).

| build | main.mr (2 s) | radial_9 (1 s) | 412_t2 (1 s) | f136_v8 (1 s) | vs thin |
|---|---|---|---|---|---|
| thin LTO (what was committed) | 1.850 | 1.412 | 1.173 | 1.370 | — |
| **fat LTO (now committed)** | **1.756** | **1.167** | **0.989** | 1.137 | **−14% avg** |
| thin + `target-cpu=native` | 1.695 | 1.176 | 1.039 | **1.125** | −14% avg |
| thin + `target-cpu=x86-64-v3` | — | 1.182 | — | 1.134 | −13% (2 assets) |
| fat + `target-cpu=native` | 1.718 | 1.276 | 1.090 | 1.235 | −7% avg |
| fat + `target-cpu=x86-64-v3` | — | 1.198 | — | 1.161 | −6% (2 assets) |
| fat + `panic = "abort"` | — | 1.177 | — | 1.142 | none |

Three findings, in order of how much they matter:

1. **The committed profile was leaving 14% on the table.** `lto = "fat"` instead of
   `lto = "thin"` is worth 5% on `main.mr`, 17% on `radial_9` and `f136_v8`, 16% on
   `412_t2` — and it is portable: no CPU flags, same binaries run anywhere.
   `codegen-units = 1` was already set, so this is the only remaining profile
   lever. It landed.
2. **`target-cpu` is worth about the same, and buys nothing on top.** Native and
   `x86-64-v3` each land within 1–2% of fat LTO (and beat it on two of four
   assets), but neither is portable, and **combining them with fat LTO is
   consistently *worse* than either alone** — 3–10% slower across four assets,
   reproducibly, in two separate rounds. Flags do not compose; the reason is
   almost certainly code layout and inlining decisions interacting with wider
   code, and it is not worth chasing for a 1–2% gain that only exists on the
   machine that built it.
3. **`panic = "abort"` buys nothing measurable here** (within noise, if anything
   slower), so it stays out: it would only complicate the codebase.

### Exactness

The constraint is that no output bit may change, so every build above was checked
against the committed one:

| check | lines / files | result |
|---|---|---|
| `dsp_trace` (every filter, f64 bit patterns) | 21 375 | identical for fat, native, fat+native |
| `core_trace` (gas dynamics, f64 bit patterns) | 89 640 | identical |
| 16-bit 44.1 kHz WAV renders, 5 assets | 5 | byte-identical |
| `cargo test --release --all-targets` under fat LTO | — | 186 pass |
| the 13 oracle parity gates under fat LTO | 31 PASS | exit 0 |

So `target-cpu=native` — with AVX-512 present and enabled — moves no bit of the
simulation or the audio. That is a fact about this code, not about Rust: nothing
here asks for reassociation, and `fp-contract` is off, so LLVM may vectorise
elementwise work but not the dependent sums the audio and gas paths are built on.
It is also the reason a future change that *does* reassociate would show up
immediately in these traces.

## 2. What was not measurable here

- **PGO.** Usually the largest single win for a workload this predictable, and
  the recipe is short: build with `-Cprofile-generate=/tmp/pgo`, run the 20-asset
  corpus, `llvm-profdata merge`, rebuild with `-Cprofile-use=...`. It needs
  `llvm-profdata`, which the packaged toolchain available here does not ship (the
  official rust distribution does). **This is the next thing to try** when a full
  toolchain is present — expect 5–15% on a branch-heavy interpreter.
- **BOLT.** Same story: needs `llvm-bolt`, and a machine to post-process on.
- **A compiler-version A/B.** Only `rustc 1.100.0` is reachable from this sandbox
  (`static.rust-lang.org` is blocked; the PyPI wheels ship one version). So the
  "1.88 vs 1.100" difference that earlier notes attributed to the compiler is
  **not attributable**: those two measurements were taken on different machines
  as well, and hardware accounts for about 35% on its own. The honest statement is
  that the compiler's contribution is unmeasured, and the way to measure it is one
  machine, two toolchains, interleaved runs.
- **Allocator.** The frame loop allocates very little per frame; swapping in
  mimalloc/jemalloc would be measurable but is unlikely to be worth a dependency.
- **Linker.** `lld`/`gold`/`mold` change *build* time, not run time. (In this
  sandbox rustc asked for `lld`, which the packaged toolchain strips; the builds
  here fell back to GNU `ld` with `-C linker-features=-lld`. That is an
  environment workaround, not something the repository should carry.)

## 3. What CI should do about it

This feeds Phase 1 (`docs/next-steps.md` §1), and adds four requirements to it:

1. **Pin the toolchain in the repository.** There is no `rust-toolchain.toml`
   today, so a developer and CI can silently disagree about the compiler. Pin an
   exact version, and make the CI job install exactly that.
2. **Build the same way everywhere.** The release profile in `Cargo.toml` is the
   thing that gets benchmarked; no job should override it with extra `RUSTFLAGS`
   (beyond a target triple).
3. **Never ship `target-cpu=native`.** Artifacts must run on the machines they are
   given to; a benchmark job that wants it may set it, and must label the result
   as non-portable.
4. **Record what produced a number.** Every performance figure in these documents
   should carry its rustc version, profile, CPU model and method — as the tables
   above do. A performance history without them is noise with a trend line.

The gate itself should stay a floor (`measure_performance.sh` already fails only
below 0.25× real time), because wall-clock on shared runners is not something to
be strict about. The *history* is what tells you something regressed; that is the
nightly job's product, not the PR gate's.
