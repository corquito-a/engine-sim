#ifndef ENGINE_SIM_PARITY_RAND_H
#define ENGINE_SIM_PARITY_RAND_H

#ifdef __cplusplus
extern "C" {
#endif

int parity_rand(void);

/* libstdc++'s <cstdlib> `#undef`s `rand`, so this header must be force-included
   *after* it (see run_core_parity.sh) for the substitution to survive. */
#define rand() parity_rand()

#ifdef __cplusplus
}
#endif

#endif /* ENGINE_SIM_PARITY_RAND_H */
