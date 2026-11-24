# Strategy: finishing the MR interpreter with minimal introduced bugs

The remaining work is `expand()`, the reference chain (`getImmediateReference` /
`getReference` / `expandChain`), `_checkReferences`, `_checkTypes` and the
operator tables. These are the phases where "port it faithfully" is most likely
to drift: their behaviour is *emergent* (memoised per context, order-dependent,
recursive), so it cannot be read off any single function, and one wrong line
changes results everywhere without changing any error code I currently assert.

This document lists every strategy I could construct, contests each one, and
then folds the survivors into the process at the end.

An honest starting point: the existing gates were designed for finished phases.
15 error fixtures only fire on *broken* scripts; the 660/701 resolved rows
record unit/definition censuses and no contexts at all; 58 script fixtures are
all valid programs. **Coverage of the unported half is currently zero.**

---

## The options

### A. Invent a positive oracle: dump the expansion table (`--expand`)

Ask the reference to print, for every structure, its `m_expansions` entries as
`X,<structure path>,<context chain>,<expansion path>`, then diff that against
the port.

*Pro:* turns "does my expansion match" into a diff; catches missing memoisation,
wrong `newChild(…, true/false)` flags and mis-gated `checkReferences` — none of
which any error fixture can reach. One turn to build.

*Contest:* I have to *guess* the observable. A dump keyed on pointers or
iteration order would be unstable; a dump coarse enough to be stable ("count of
expansions") would pass while the port is wrong. It also only covers what the
58 corpus scripts exercise, and those are all well-formed engine units — they
never stress the recursive corners. Cost of a wrong dump: I build a gate that
cannot fail, which is worse than no gate.

*Verdict:* keep, as a **structural** gate, but never as the only one. Require a
canary (see the admission policy) before trusting it.

### B. Mutation fuzzing of the corpus

Mutate the 58 valid scripts (token-level: rename, delete, duplicate, reorder),
run both implementations, diff the error codes.

*Pro:* cheap, needs no new oracle, and explores inputs nobody wrote by hand.

*Contest:* most mutations land in the lexer/parser, which is already 100% and
already gated by 24 IR tests — so the yield per mutated file is low. Worse, the
diff is *asymmetric*: on the unported half my port reports nothing, so a
divergence only tells me "unported", not "wrong". Roughly: this finds bugs in
the phases I have already finished, which the corpus and fixtures already cover.

*Verdict:* keep as a **low-cost regression net** over the ported half, once the
whole pipeline is done. Do not use it as the primary gate now.

### C. Live co-simulation: call the C++ from the Rust tests

Build the reference as a shared library with a C ABI and have the test suite
call it, so every test compares live instead of against recorded files.

*Pro:* the strongest possible answer to fixture staleness — thousands of
comparisons, and expectations can never drift from the code.

*Contest:* it introduces an ABI boundary, cross-language arena lifetimes
(`TRACK`/`FTRACK` ownership), exception containment, and a build dependency on a
compiler for `cargo test` — for a problem I do not actually have. The fixtures
have only ever changed when I *deliberately* extended a dump. It also makes the
Rust crate unbuildable without the submodule, which is the property that has
made this port easy to resume across sessions.

*Verdict:* defer indefinitely. Revisit only if fixture maintenance ever exceeds
about one turn a month.

### D. Metamorphic properties (oracle-free)

Assert relations that must hold without knowing the answer: renaming every
definition preserves the error-code multiset; reordering independent definitions
preserves it; expanding twice equals expanding once; adding an unrelated
definition changes nothing about the others.

*Pro:* no reference needed, so it runs everywhere and catches order-dependent
and stateful bugs — a real class in a memoised, context-keyed expansion.

*Contest:* metamorphic relations are usually *weaker* than they look. "Expanding
twice equals once" is trivially true in my implementation because of the memo
being a map rather than a vector — i.e. it verifies my code is self-consistent,
not that it matches the original. The rename/reorder properties mostly exercise
name resolution, which is already gated.

*Verdict:* keep two of them (idempotence of `expand`, invariance under
declaration reordering) as cheap invariants; reject the rest as theatre.

### E. Make bugs unrepresentable rather than tested

Shape the API so the failure modes cannot be written: one `memoise(context) ->
&mut Option<StructureId>` entry point with no other write path, `ContextId` as an
opaque newtype that cannot be compared to a `StructureId`, `ExpansionTable`
private by construction.

*Pro:* the highest bug-prevention per line of code, and it is permanent — it
covers all future phases (M3 will walk these same contexts).

*Contest:* it cannot validate *fidelity*. A beautifully typed model of the wrong
recursion is still the wrong recursion. It also has an upfront cost precisely
when I am least sure of the shape, and premature abstraction here would mean
rewriting it once I see the real call graph.

*Verdict:* adopt as the *code shape* for the new modules, introduced as each
phase lands (not up front), and never counted as a gate.

### F. Coverage matrix synthesised from the rules registry

Derive the fixture space mechanically from `LanguageRules` (80 builtin types,
27 with channel types, the port-type forms, visibility values) and generate one
recording per cell.

*Pro:* provable coverage instead of hoped-for coverage.

*Contest:* the full cross-product is enormous for very little marginal
information — 80 × forms × visibility is thousands of recordings, most of which
exercise the same code path. The genuinely orthogonal axes are far fewer (about
10 construct classes), and the corpus plus fixtures already hit most of them.

*Verdict:* adopt in reduced form — a **construct census** test asserting, per
construct class (named/positional ports, alias chains, defaults, visibility,
nested instantiation, operators, imports), that at least one fixture exercises
it. Reject the full cross-product.

### G. Trace diffing (call-sequence parity)

Instrument the reference to log every `_expand` / `getImmediateReference` call
with its arguments; diff the call sequences rather than only the results.

*Pro:* when outputs diverge, it localises the *first* divergent call instead of
leaving me to bisect a recursion by hand.

*Contest:* it puts debug output in the reference and matching trace points in
the port, i.e. it couples the implementation to a diagnostic format, and it
produces traces large enough that diffing them is its own investigation. It is
genuinely useful exactly once — when a divergence resists explanation — and it
is cheap to add at that moment.

*Verdict:* build on demand, never preemptively.

### H. Replay the original's own test suite

There are **113 `.mr` fixtures and 59 gtest assertions (46 of them asserting an error code *and its line number*)** in
`dependencies/submodules/piranha/test/` that nothing in this project has ever
run:

* `test/sdl/general-tests/` (40 files) with expectations like
  `findError(errList, ErrorCode::UnresolvedReference, 18)` and
  `EXPECT_EQ(errList->getErrorCount(), 1)`;
* `operator-tests/` (11) which assert *reference channel types* from
  `getReference(query)` — the exact machinery I am about to port;
* `infinite_loop_test_1..4.mr`, whose self-referential attributes are the
  `expandChain` loop guard;
* `visibility-tests/`, `resolution-tests/`, `construction-tests/`,
  `optimization-tests/`, `full-error-testing/`.

The inputs exist (113 fixtures, 59 test cases), the assertions exist, and they cover precisely the phases with
zero coverage — including error *line numbers*, a dimension this port has never
been held to.

*Pro:* an authoritative executable spec, for free, aimed at the unported half.

*Contest:* three real problems. (1) They are written against `TestRules`, a
different `LanguageRules` subclass with its own builtin set, so the oracle needs
a second rules mode and I must not confuse "piranha is wrong here" with "the
test's toy rules differ". (2) Some tests assert runtime behaviour
(`NodeProgram::execute`, `fullCompute`), which belongs to M3 and cannot run yet.
(3) `EXPECT_EQ(getErrorCount(), 1)` makes the comparison *strict* — extras
fail — and necessarily so, which is exactly the property I want but also means
one spurious diagnostic blocks a whole fixture.

*Verdict:* adopt, and make it the primary gate for the unported half. The
contest points are all containment problems with known answers (a second oracle
mode, a skip-list for runtime-only tests, strict counts on purpose).

---

## Comparison

| Option | Covers unported half | Cost | Failure mode if adopted alone |
|---|---|---|---|
| A `--expand` dump | structure, on valid input | 1 turn | stable-but-vacuous dump |
| B mutation fuzz | little (parser mostly) | cheap | tests the finished half |
| C FFI co-sim | yes, broadly | high | slow, unbuildable-without-submodule |
| D metamorphic | little | very cheap | self-consistency mistaken for fidelity |
| E typed API | n/a (prevention) | medium, ongoing | wrong model, immaculately typed |
| F construct matrix | partially | medium–high | combinatorial waste |
| G trace diff | yes (diagnosis) | medium | couples code to debug format |
| H replay gtest suite | **yes, targeted** | **1–2 turns** | toy-rules confusion, strict counts |

The decisive observation is in the last row: **H provides the expectations I was
about to invent for A**, and provides them in the phases that currently have no
coverage at all. When an oracle already exists, inventing one is the more
expensive path to a weaker gate.

---

## The strategy

Ordered by what it buys per turn, with the deferred items explicitly parked.

**Tier 0 — construction (E, in reduced form).** New modules expose exactly one
write path for the memo and never take a raw `usize` where a context is meant.
Introduced as each phase lands, not up front.

**Tier 1 — replay the original's suite (H).**
1. `script_reference --test-rules [--errors] [--lines]`: link
   `test/test_rules.cpp` behind a one-line gtest shim, resolve fixtures relative
   to `test/sdl/`, and print `code,line` per error.
2. Record every fixture's expectation into `rust/tools/parity/spec/` (one file
   per fixture, plus a skip-list naming the runtime-only tests and why).
3. Hold the port to `(code, line)` multisets *and* error counts, for all
   fixtures that reach the compilation phases.

**Tier 2 — structure (A).** `--expand` dump, keyed on semantic identity
(structure path, context chain), recorded from the reference and diffed against
the port. Second opinion on the same phase; catches the memo/context bugs that
only change *which* node a reference lands on.

**Tier 3 — net.** Keep the corpus (58 scripts), the 15 error fixtures, the
resolved rows, and the two surviving metamorphic invariants (D). Add the
construct census (F, reduced).

**Parked with a trigger.** C if fixture maintenance exceeds ~1 turn/month; G the
first time a divergence resists explanation; B once the pipeline is complete.

## The process

1. **One phase, one commit, gate first.** The unit of work must be smaller than
   the gate that verifies it. When a phase needs two files to make sense
   (`expand` + `getImmediateReference`), the gate lands first describing both.
2. **Gate admission policy.** A gate is admitted only if it has (a) a *canary* —
   a deliberate perturbation of the port that demonstrably turns it red, (b) a
   named bug class it catches, and (c) a runtime under about a minute. A gate
   that has never been red is not yet a gate.
3. **Bug-class ledger.** Every bug found is entered with the gate that caught it
   and the gate that *should* have. Bugs with no assigned gate are the argument
   for the next gate; gates that never catch anything are removed.
4. **Probe, don't reason.** Any question about the original's behaviour is
   answered by a two-line `.mr` file through the oracle, never by inference.
   Both times I trusted intuition over the C++ this session, intuition was wrong
   (the 69-of-81 builtin scrape; the `isInstantiationError` expectation).
5. **Never widen a tolerance to make progress.** `RESOLVED_GAP`, skipped rows and
   "close enough" counts all hide exactly the bugs the gate exists for.
6. **Prefer execution to extraction.** Where a table must cross from C++ to Rust
   (operator rules, builtin reference nodes), record it by running the reference,
   not by scraping source — and assert completeness (count scraped == count
   registered) so a partial extraction fails loudly.

## Immediate sequence

1. Oracle: `--test-rules`, line numbers, gtest shim (~1 turn).
2. Record all 59 non-runtime fixtures; triage any that fail under `TestRules`.
3. Port `expand`/`m_expansions`, then `getImmediateReference`, held to
   `(code, line)` for the fixtures that reach those phases.
4. Port `expandChain` + `InfiniteLoop` against `infinite_loop_test_1..4.mr`.
5. `_checkReferences`, `_checkTypes`, operator tables — each with its fixtures.
6. Then M3 (node generation) — where the runtime-only tests finally become
   usable, and the same fixtures get promoted from skip-list to executed.

---

## Hybridisation: measured, not assumed

"Use more than one option to be sure" is right in spirit and wrong in
arithmetic. Confidence multiplies only across *independent* failure modes. Two
gates that look at the same observable, or that both need the port to be right
in the same way, share their blind spots and add nothing.

So the question was measured rather than argued.
`rust/tools/mutation_check.py` applies one deliberate fault at a time to the
ported circularity phase, runs the suite, and reports which gate notices. First
run: **3 of 6**. That is the useful outcome — it located gaps instead of
confirming the tests.

| Mutation | First run | After the remedy | Remedy |
|---|---|---|---|
| cycle never reported | caught (unit + fixtures) | caught | — |
| components gated on the inverted flag | caught (unit + fixtures) | caught | — |
| walk never follows a node into its definition | caught | caught | — |
| instantiation frames newest-first | **missed** | caught | 3-cycle fixture asserting frame *order*, not count |
| no guard against re-entering the context chain | **missed** | caught (hang/overflow, no failing test name) | `circular_guard.mr`: a definition reached from a root that re-enters itself |
| main context used where a plain one belongs | **missed** | **still missed** | call site now reads `main_child(…)`, so the boolean cannot be swapped at all |

Three lessons, in order of importance:

1. **The corpus and the resolved gate saw none of this.** They are invariant
   under all six faults, by construction: every corpus script is acyclic and
   well-formed. That is the concrete argument for option H over "we already have
   58 scripts and 660 resolved rows" — volume of *passing* fixtures is not
   coverage of *failure modes*.
2. **Overlap is visible in the results.** The three caught mutations are caught
   by the same four unit tests. Adding a fifth gate over the same observable
   would not change the column. Independence, not count, is what to buy.
3. **One fault was unobservable, and the answer was not another gate.** The
   main-context flag is carried for operator expansion (`touchedMainContext`
   gates whether `InvalidOperandTypes` may be reported), which is not ported, so
   no assertion in the finished phases could see a swap — inventing one would
   have tested my implementation against itself, not against the original. The
   remedy is a shape change: the call site says `main_child(…)`, so the fault
   can no longer be expressed. Prevention where observation is impossible;
   fixtures where it is possible.

### The hybrid that survives this filter

| Layer | Option | What only it can see |
|---|---|---|
| semantics | H (original fixtures, `(code, line)` + counts) | wrong diagnostics on constructs the corpus lacks |
| structure | A (`--expand` dump) | *which* node a reference lands on: memo, context identity, `checkReferences` gating — invisible to codes and to censuses |
| shape | E (`main_child`/`child`, single memo write path) | faults that no assertion can reach, by making them unrepresentable |
| meta | canary + this mutation check | gates that cannot fail |

Deliberately excluded, with the threshold that would bring each back:

* **C (live co-simulation)** — only if fixture maintenance exceeds ~1 turn a
  month. Not reached: fixtures have changed only when I extended a dump.
* **G (trace diffing)** — the first divergence that resists explanation by
  reading. Then it is a one-turn investment with a clear target.
* **B (mutation fuzzing of scripts)** — once the pipeline is complete, as a
  net over `(code, line)` for inputs nobody wrote by hand.
* **F (full construct matrix)** — reduced to a census; the cross-product is
  combinatorial waste.

**Working rule:** a phase is finished when `mutation_check.py` reports every
mutation caught *or* explained (unobservable, with the shape changed so it
cannot recur). That replaces "the tests pass" with "the tests would have failed".
