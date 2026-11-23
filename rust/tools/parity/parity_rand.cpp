// Deterministic stand-in for the C library's global `rand()`.
//
// The combustion model draws one random number per ignition event
// (`CombustionChamber::ignite`), which makes the original non-reproducible
// across platforms. For parity testing, `combustion_chamber.cpp` is compiled
// with `-Drand=parity_rand` and linked against this file, which implements the
// same 31-bit LCG as the Rust port's `fuel::Rng`.
//
// `crates/engine-sim-core/src/fuel.rs` uses a state of 1 and the recurrence
//   state = (state * 1103515245 + 12345) & 0x7fffffff
// returning `state`; the C++ side then divides by RAND_MAX exactly as before.

#include <stdint.h>

static uint64_t s_state = 1;

extern "C" int parity_rand(void) {
    s_state = (s_state * 1103515245ULL + 12345ULL) & 0x7fffffffULL;
    return (int)s_state;
}
