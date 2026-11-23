//! Native Rust port of the Engine Simulator simulation core.
//!
//! This crate is a faithful transliteration of the C++ sources in `src/` and
//! `include/` at commit `85f7c3b` of `ange-yaghi/engine-sim` (MIT).
//!
//! # Fidelity rules
//!
//! The simulator's output (engine audio, dyno curves, gauge behaviour) is
//! emergent from thousands of small arithmetic operations with hand-tuned
//! constants. There is no specification to code against, so a port is only
//! correct if it produces the *same numbers* as the original. Every module in
//! this crate therefore follows these rules:
//!
//! 1. **Same operation order.** Expressions are transliterated left-to-right
//!    exactly as they appear in C++; no algebraic refactoring, no hoisting that
//!    changes rounding.
//! 2. **No FMA contraction.** Rust does not enable floating-point contraction
//!    by default; do not enable it, and do not use `mul_add` in ported code.
//! 3. **Same primitives.** `std::pow` → `f64::powf`, `std::sqrt` → `f64::sqrt`,
//!    `std::fmin/fmax` → `f64::min/max`. Transcendentals go through the same
//!    libm on the same platform, so results are bit-identical against a
//!    C++ build on that platform.
//! 4. **Same defaults.** Constructor/parameter defaults (including `Mix`'s
//!    `{0, 1, 0}` and `degreesOfFreedom = 5`) are preserved.
//! 5. **Every function cites its C++ original** so the two can be diffed by
//!    eye.
//!
//! Verification lives in two places:
//!
//! * `tests/` — the upstream gtest cases ported to `#[test]`.
//! * `rust/tools/parity` — a harness that compiles the *original* C++ sources
//!    with `g++` and diffs raw `f64` bit patterns against `examples/gas_trace.rs`
//!    over a scripted scenario. See `rust/README.md`.

pub mod chamber;
pub mod constants;
pub mod drivetrain;
pub mod engine;
pub mod fuel;
pub mod function;
pub mod gas;
pub mod ignition;
pub mod parts;
pub mod scs;
pub mod simulator;
pub mod units;
pub mod utilities;

pub use chamber::CombustionChamber;
pub use engine::Engine;
pub use fuel::Fuel;
pub use function::Function;
pub use gas::{FlowParameters, GasSystem, Mix};
pub use parts::{
    Camshaft, ConnectingRod, Crankshaft, CylinderBank, CylinderHead, ExhaustSystem, Intake, Piston,
};
pub use scs::{RigidBody, RigidBodySystem};
pub use simulator::Simulator;
