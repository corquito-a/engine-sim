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

---

## First run of Tier 1 (the original's own suite)

Implemented: `stage_script_reference.sh` now stages `piranha/test/test_rules.cpp`
(plus an empty `<gtest/gtest.h>` shim — no gtest here), the oracle takes
`PARITY_RULES=test` to select those rules and `PARITY_LINES=1` to append the
source line, and `ir_dump --lines` emits `(tok "text" specified line)`. The IR
round-trip grammar is untouched: lines are opt-in on both sides.

**Verified end to end.** `general_syntax_test_1.mr` → `R0050,18`, one error.
The original's own assertion is
`findError(errList, ErrorCode::UnresolvedReference, 18)` with
`EXPECT_EQ(getErrorCount(), 1)`. The Rust parser, the line-carrying dump, the
`TestRules` oracle and the `(code, line)` comparison all agree with it.

Three findings, all of them the kind that only a conformance suite produces:

1. **Operator errors lose their line.** `general_syntax_test_2.mr` reports
   `E0010,0` where the suite asserts line 26. Cause: `Value::Binary`/`Unary`
   carry no operator token, so the dump has nothing to carry a line on and the
   harness leaves the summary token default. Fix: add the operator token to the
   IR (parser → lower → dump → harness), then expect `E0010,26`.
2. **A real parser divergence.** `general_syntax_test_4.mr` fails to parse in
   Rust (`16:18: expected a value`) where the original accepts it and complains
   at 24/26 with `UnexpectedToken`. The oracle can only run fixtures the Rust
   parser accepts, so this is a parser bug with a ready-made fixture.
3. **Parser-error fixtures cannot use the oracle at all** — if my parser
   rejects the file there is no dump to rebuild. Those stay with the parser
   tests; the conformance suite covers phases *after* the parser.

Both open items are now gated by fixtures rather than by memory, which is the
point of Tier 1: the suite is not a score, it is a to-do list with line numbers.

## The conformance report (first full run)

`python3 rust/tools/parity/run_conformance.py --summary`, over all 113 fixtures:

```
DIFF 54, MATCH 40, PARSE 19
```

40 fixtures already reproduce the reference's `(code, line)` list exactly. The
54 disagreements group by missing code, which is the to-do list:

| Code | Meaning | Missing | Phase it belongs to |
|---|---|---|---|
| `R0010` | undefined node type | 29 | resolution (unexpected -- needs triage) |
| `R0060` | undefined member | 19 | reference chain |
| `R0050` | unresolved reference | 18 | reference chain |
| `E0010` | invalid operand types | 9 | operator expansion |
| `T0010`/`T0011` | type errors | 5 / 4 | `checkTypes` |
| `R0080` | circular reference | 3 | `expandChain` loop guard |
| `V0080`/`V0081` | undefined builtin input/output | 1 / 1 | `validateBuiltinMappings` (known) |
| `E0020`, `R0061`, `V0010`, `V0011`, `V0040`, `R0020`, `R0030`, `R0040` | assorted | 1-3 each | mixed, triage per fixture |

`R0010` at 29 was the surprise, and triage says it is not a port bug: the
reference reports those on an *unspecified* token (line 2147483647) because
`IrValueLiteral::_expand` builds an `IrLiteralNode` whose definition is looked up
in the unit (`resolveBuiltinNodeDefinition`), and a fixture that does not define
`__piranha__literal_*` itself makes the expansion resolve the empty type name --
piranha's own `// TODO: raise error, literal type not defined`. Faithful
behaviour, unreachable in the port until expansion exists.

Identified by reading the stack rather than the code: instrumenting the two
`UndefinedNodeType` sites with `backtrace` showed the caller chain
`IrValueLiteral<...>::_expand → IrNode::_resolveDefinitions →
resolveNodeDefinition`, with `type=''`. The same check on `es/engine_sim.mr`
reports nothing, because the engine's own stdlib defines its literal builtins.

**After excluding unspecified-token errors: `MATCH 57, DIFF 37, PARSE 19`
(171 unspecified-token errors dropped).** Every surviving gap belongs to an
unported phase -- `E0010` 9 and `R0080` 3 (operator expansion, `expandChain`),
`T0010`/`T0011` 9 (`checkTypes`), `V0080`/`V0081` 2 (`validateBuiltinMappings`),
`R0061` 1 (visibility) -- except `V0010` 3, `V0011` 1, `V0020`-class argument
errors `R0020` 3, `R0030` 2, `R0040` 2 and `E0020` 2, which belong to phases
that *are* ported and are therefore real bugs. That short list is the next
work queue, and it is short enough to work through fixture by fixture.

Two fidelity fixes came out of the triage. The harness now always writes
`m_typeInfo` (`IrAttributeDefinition`'s constructor initialises only
`m_typeDefinition`, so an untouched `m_typeInfo` is uninitialised memory). And
the runner excluded unspecified-token errors by default, `--strict` kept them --
an exclusion that has since been retired, because it turned out to be hiding a
port gap rather than an oracle artefact: "The unspecified-token errors" below.

`PARSE` 19 was listed here as the parser's own to-do list. That reading turned
out to be wrong -- they are fixtures whose expected errors come from bison, or
whose import target is not in the repository, and neither is a port defect. "The
19 fixtures that cannot be scored" below has the audit.

Supporting pieces added with the report: `Rules::{EngineSim, Piranha}` on the
arena (selectable through `error_dump --rules=piranha`), the 37-name
`PIRANHA_BUILTIN_NODE_TYPES` table, and a test that rescans `test_rules.cpp`
and fails if the table and the source disagree -- transcription checked against
its origin rather than trusted.

## The line-number mismatches, fixed

The fourteen ported-phase errors left in the report (`V0010` 3, `V0011` 1,
`R0020` 3, `R0030` 2, `R0040` 2, `E0020` 2) are all **locations**, not codes or
counts. Diagnosis, done by experiment rather than by reading:

* `attribute_definition_test.mr` reports `R0040,1 R0030,1`; the attributes are on
  lines 13-16. Moving them 40 lines down leaves the report at `...,1`, so the
  location is not the attribute's own token.
* `CompilationError` copies its location verbatim from the token it is handed,
  and that token is `attribute->getSummaryToken()`. `registerToken` folds each
  registered token into the summary with `combine`, which keeps the **minimum**
  `lineStart`. So the summary is `min(name line, any other registered token)`.
* The other registered token is the attribute's *value*. In the dump a string
  literal is `(str "kaboom")` -- no token, no line -- so the harness synthesises
  `IrTokenInfo_string(value, 1, 1, 1, 1)`, i.e. **line 1**. The summary becomes
  `min(15, 1) = 1`.

So the reference's `...,1` is an artefact of my dump not carrying literal
tokens, not the original's semantics: with the real parser the literal has its
own token on line 15 and the summary would be 15.

The mirror-image cases have the same root: `V0040,0` in the reference comes from
a summary that folded a *default* token (line 0 in the dump), while the port
reports the entry's own summary line.

**Root cause, one sentence:** neither side models `IrParserStructure::combine`
for summary tokens, and literal values carry no token through the dump.

**Fixed in that order.** Constants now carry their token through the IR
(`Value::Int/Float/Bool/Str`), a literal's summary *is* its token (as
`IrValueConstant`'s constructor does with `useToken`), the dump emits
`(int (tok "5" 1 4) 5)` in lines mode, the harness builds constants with that
token and gives unspecified tokens the default `INT_MAX` span, and
`resolve_all_definitions` walks units deepest-first because
`IrCompilationUnit::resolveAll` resolves imports before itself.

Result: `MATCH 61, DIFF 33` (from 57/37), and the ported-phase residual is down
from fourteen errors to five, in three fixtures -- `general_syntax_test_36.mr`
(`V0010`) and `infinite_loop_test_3.mr` / `infinite_loop_test_4.mr` (`E0020`),
all of which report a *different* token than the reference rather than a
different code.

**Original plan, for the record:**
1. give constant values a token in the IR (`Value::Int/Float/Bool/Str`), the same
   way the operator token was added, and emit it in lines mode;
2. implement `combine` for summary tokens in the port (`min` lineStart, `max`
   lineEnd, first column wins on ties), and have the harness build the same
   summary for nodes and attributes;
3. re-run: the fourteen should collapse, and any that do not are real.

Note what this is *not*: none of the fourteen is a wrong diagnostic. Chasing the
codes would have found nothing; comparing lines is what exposed it.

---

## Ported-phase residuals: zero

After two more fixes the conformance report no longer contains a *single*
difference in a phase that is ported. `MATCH 63, DIFF 31`:

| Code | Missing | Phase |
|---|---|---|
| `R0060` undefined member | 19 | reference chain |
| `R0050` unresolved reference | 18 | reference chain |
| `E0010` invalid operand types | 9 | operator expansion |
| `T0010` / `T0011` type errors | 6 / 4 | `checkTypes` |
| `R0080` circular reference | 3 | `expandChain` loop guard |
| `V0080` / `V0081` builtin in/output | 1 / 1 | `validateBuiltinMappings` |
| `R0061` internal member | 1 | reference chain |

Everything left is work, not drift. The two fixes:

1. **A token child is a `(tok ...)` form, not just any list.**
   `(un neg (float 459.67))` was read as "the operand is a token", so the value
   base moved to 3 and `--ir` read past the end of the form: a segfault for the
   whole corpus. Found only because the staged reference was rebuilt from
   scratch; the previous green run had used a cached binary. The parity script's
   rebuild is now part of every run here, and `--keep` is what leaves the oracle
   around for the conformance runner.

2. **A specified token with no position is `INT_MAX`, not 0.**
   `IrNode::setThis` synthesises `IrAttribute(IrTokenInfo_string("this"), ...)`,
   whose span ctor leaves the base default -- `lineStart == INT_MAX`. The dump
   wrote 0 for the same token, so the oracle folded a line 0 into every summary
   it touched and reported `V0010,0` where the port said 12. The dump now writes
   `INT_MAX` for a specified token with no position, and an unspecified token
   maps back to the default span on the way in.

Both were *harness/dump* fidelity, again. The pattern is consistent enough to be
a rule: when the reference and the port disagree only about a position, suspect
the dump before the port -- twice now the reference's number was derived from a
token the port had synthesised or dropped.

---

## The reference chain, and the measurement that says "not yet"

`reference.rs` now implements `getImmediateReference`/`getReference` for labels,
literals, attributes, ports, nodes and operators, with the chain, the
`m_infiniteLoops` marking and the `touchedMainContext` rule that decides whether
a failure is worth reporting at all. It is **not** wired into the pipeline: the
pipeline order is `resolveDefinitions -> checkCircularDefinitions -> expand ->
checkInstantiation -> checkTypes`, and expansion is what makes most references
resolve.

The measurement, with `SCRIPT_CHECK_REFERENCES=1` switching the phase on:

| | ported phases only | + reference phase |
|---|---|---|
| MATCH | 63 | 43 |
| R0050 missing | 18 | 1 |
| R0061 missing | 1 | 0 |
| R0060 missing | 19 | 12 |

The reference phase finds nearly all of the original's unresolved references --
and invents 20 fixtures' worth of its own, because a reference through a literal
or an operator cannot resolve until `expand` has produced the node it points at:
`(label "q")` dead-ends, the original's `q` had an expansion. The invented ones
are `R0050`/`R0060`/`R0080` on scripts where the original reports nothing at all
(`general_syntax_test_17`, `_25`, `_26`, `_27`, `_29`...).

So the phase stays opt-in until expansion lands. This is the useful kind of
negative result: it says the machinery is structurally right (R0061 matches
line-for-line, most R0050/R0060 land on the right line) and that the *only*
thing between it and the gate is expansion, not more reference logic.

*(Expansion, the reference phase and the type check all landed; the numbers
moved to `MATCH 87, DIFF 7`. See "The reference phase, and the two rules that
make it agree" and "The type check, in three early exits" at the end of this
document.)*

---

## The language rules are generated, not transcribed

Expansion asks the rules four questions -- which builtin node stands behind a
literal, behind `left op right`, behind a unary operator, and behind a
conversion -- and the answers are 4 + 12 + 2 + 3 calls in
`scripting/src/language_rules.cpp` and 4 + 17 + 7 + 3 in
`piranha/test/test_rules.cpp`.

`rust/tools/parity/extract_language_rules.py` parses those two files and writes
`src/language_rules.rs`; every call site has to produce exactly one row, so a
parse that stops early fails instead of writing a short table.
`tests/language_rules_tests.rs` then re-derives both tables from the C++ with an
independent parser and compares them row for row -- the same guard the
builtin-node table uses, and the reason `--check` has a second caller in CI.

One table in the same file cannot work that way -- the builtin reference nodes'
ports, which live in C++ constructors rather than in a declaration the script
could parse. Those are recorded from the running original instead; see "The
builtin port table is recorded, not transcribed" at the end of this document.

Two details of the lookup that the table has to respect:
`resolveOperatorBuiltinType` returns `""` when *either* operand type is null
(unlike `checkConversion`, which returns false), and the lookup **is**
reversible: `OperatorMapping::operator==` falls back to comparing
`leftType`/`rightType` swapped (`include/language_rules.h:54-75`), and
`reversible` defaults to `true` with neither ruleset ever passing a fourth
initialiser. So `int + float` matches the registered `Add(Float, Int)` rule.
Getting that wrong is not academic: it is one of the causes below.

---

## Expansion: the first reading, and why it was wrong

`expansion.rs` reproduces both entry points (`expand()` with its per-structure
flag, `expand(context)` with its `m_expansions` memo), the unit's and the node's
`_expand`, `expandChain`, and the five overrides that create something: literal
nodes, interface nodes for untyped ports, conversion nodes, and unary/binary
operation nodes whose inputs are `IrInternalReference`s.

The first measurement, behind `SCRIPT_EXPAND=1`, was `MATCH 62` against `63` for
the pipeline without it, so it changed nothing but the `E0010` line numbers, and
the reading at the time was that the report condition
(`touchedMainContext || emptyContext`) and the dead-end rule
(`isFixedType`/`isStaticType`) were being resolved the wrong way round in places.

That reading was wrong. Both conditions were already right; what was wrong was
the *inputs* they were given. The cause turned out to be a set of small,
independent divergences, each of which made an operand type unknown or a
reference spuriously failed -- and each of which was found by shrinking a
failing fixture to five lines rather than by reading the fixtures.

A sub-observation from that phase is still worth keeping: the unit's `_expand()`
-- all components against a unit main context -- is what expands an instance's
attributes and therefore its literals, so leaving it out makes every literal
lookup fail in the unit context.

---

## Four causes behind the `E0010` family, and expansion becomes the pipeline

`E0010` (invalid operand types) was the whole of the expansion mismatch: nine
fixtures in which the original reports it and the port does not, and a handful
where the port reports it twice where the original reports `R0050`. Four
independent causes, each a faithful-looking simplification of the original:

1. **`m_expansions` is keyed by context *value*.** `PKeyValueLookup<IrContextTree,
   IrNode *>` compares keys with `*stored == *key`, i.e. `IrContextTree::isEqual`,
   not by pointer. The port keyed the map by context id, so the moment a walk
   created an equal-but-distinct child -- which is exactly what a node's
   `getImmediateReference` does on every call -- every expansion recorded under
   the original context became invisible, and operands had no type.

2. **`resolveChannelType` is per ruleset.** The port had one hand-written table
   of `__engine_sim__*` builtins, so under the fixtures' rules
   `__piranha__literal_int` had no channel type at all. Both tables are now
   generated from the two `registerBuiltinType` bodies (27 engine-sim, 10
   piranha), checked by the same independent re-parse, and the hand-written
   duplicate is gone -- one source of truth, not two.

3. **The operator lookup is reversible** (see above). `5 + 5.0` is an
   `Int + Float` query, the piranha rules only register `Add(Float, Int)`, and
   the original resolves it by comparing the operands swapped. Without that the
   port reported `E0010` on `operator_test_2.mr`, which the original accepts.

4. **`IrAttributeDefinition` makes every port public.** Both its constructors
   call `setVisibility(IrVisibility::Public)`; `IrParserStructure`'s own default
   is private. The port left ports at the default, so any `a.b` through a typed
   input port failed as `AccessingInternalMember` before the operator could even
   ask for a rule -- the operand type was never computed.

Two more, found on the way, that were not about types at all:

* **`IrValueNodeRef` has an immediate reference.** `getImmediateReference`
  returns the node the value stands for (`include/ir_value_constant.h:205`), and
  `getAsNode` unwraps it too. The port treated it as having none, which turned an
  `Int + Float` into a silent dead end.
* **A nested `getReference` shares the expansions.** The dot operator's basic
  query starts a fresh *chain*, but the port had also given it a fresh
  `ReferenceState`, i.e. empty expansions -- so `a.b` inside a definition could
  not see the interface node expansion of `a`. The `ReferenceState` is now a
  field on the `Arena`, shared by every phase, because in the original
  `m_expansions` lives on the structures.

The last one also fixed a gate's runtime: grouping the expansions per structure
(as the original's per-structure maps do) turned an O(n²) scan into a lookup, and
`tests/error_tests.rs` -- which now expands, because `resolveAll` does -- went
from 551 s to 19 s.

**Expansion is on by default now.** Measured, on top of the merge of #13:

| | default | expand (new default) | + reference phase |
|---|---|---|---|
| MATCH | 63 | **69** | 57 |
| DIFF | 31 | **25** | 37 |
| `E0010` | 9 | **0** | 0 |

The promotion was justified the only way it can be: `expand` fixed five fixtures
and regressed none (`operator_test_2.mr` was the one regression until the
reversibility bug was fixed, and it is gone).

## The reference phase, and the two rules that make it agree

`checkReferences` (`_checkInstantiation`) is now part of the pipeline too, and the
phase went from inventing errors on 19 fixtures to `MATCH 83, DIFF 11`:

| | expansion only | + reference phase |
|---|---|---|
| MATCH | 69 | **83** |
| DIFF | 25 | **11** |
| `R0050` missing | 18 | 0 |
| `R0060` missing | 19 | 5 |
| `R0080` missing | 3 | 1 |

Two causes, both found by shrinking a fixture to four lines:

1. **`m_expansions` outlives the phase.** The original keeps it on the
   structures, so `checkReferences` reads back what `expand` wrote; the port
   created a fresh `ReferenceState` per phase. It is now an `Arena` field, and
   the table is grouped per structure, as the original's per-structure maps are
   (which is also what turned an O(n^2) scan into a lookup: `error_tests` went
   from 551 s to 19 s).
2. **`auto node X` instantiates a node named `X`** (`immediate_node_definition:
   AUTO specific_node_definition`), before registering the definition, and
   `IrCompilationUnit::resolveLocalName` finds *nodes*. Without the name, a
   reference to `X` from inside `X`'s own body resolved in the original and not
   in the port.
3. **The dot's right operand is not a reference.**
   `IrBinaryOperator`'s constructor calls `setCheckReferences(false)` on it --
   "the right-hand operand will *always* be a standard label that doesn't
   reference anything itself". Without that, every `a.b` reported a spurious
   `R0050` on `b`, which was most of the over-reporting.
4. **An attribute's values see the node's *scope parent*, not the node.**
   `IrNode::setScopeParent` forwards the node's parent scope to its attribute
   list, so the walk from a label is `label -> attribute -> attribute list ->
   (node's parent)`. Leaving the node in that chain made `i(x: 5, y: x)` resolve
   `x` to the attribute and report a *circular reference* where the original
   reports an unresolved one.

What is left after this phase was the unported `checkTypes` (`T0010`/`T0011`,
four fixtures) and the `validateBuiltinMappings` tail (`V0080`/`V0081`, one
fixture); the next three sections close both, and the last of them closes the
reference-phase residuals too. The gate ends at `MATCH 94, DIFF 0`.

The lesson that keeps repeating: the failing fixture mixes several questions at
once. Five-line probes answer them one at a time in minutes, and every cause
above was found that way -- not by reading the fixtures that failed.

## The type check, in three early exits

`checkTypes` is the last phase `resolveAll` runs, and it is the whole of the
`T0010`/`T0011` family. Ported as `types.rs`, it takes the report to
`MATCH 87, DIFF 7`:

| | + reference phase | + type check |
|---|---|---|
| MATCH | 83 | **87** |
| DIFF | 11 | **7** |
| `T0010` missing | 6 | 0 |
| `T0011` missing | 4 | 0 |

No new diffs: the four fixtures that carried them
(`general_syntax_test_11`, `_12`, `_21a`, `_21b`) now agree line for line.

The phase has two entry points and both matter. `checkTypes()` walks every
structure and, on a *node*, re-checks that node's definition against a fresh
main context -- that is how a definition body gets checked once per
instantiation, with the per-structure `m_typesChecked` flag keeping it to once.
`checkTypes(context)` walks the same tree with one context, which is how the
unit's main context reaches a definition body at all. The unit's own
`_checkTypes` makes one main context; a node's makes a non-main root with a
main child focused on the node.

Three early exits are what make the phase agree, and all three are copied
verbatim rather than tidied:

1. **A reference that only touched the main context** is checked against the
   unit's context; one that did not is skipped unless the context is empty.
   `IrAttributeDefinition::_checkTypes` has a second gate on top of it --
   empty *and* main -- which `IrAttribute` does not have.
2. **A static type is only interesting in an empty context.** A static type is
   known at the definition site, so there is nothing to compare at an
   instantiation that already resolved.
3. **A type that is already known to be right is not a candidate.** A fixed
   type equal to the port's type definition returns before any conversion
   table is consulted, and so does a resolved node whose `getAliasType()`
   equals it. Only then do the channel types get compared, and only then is
   `checkConversion` asked.

Two asymmetries are deliberate. The error token differs:
`T0010` is reported at the *attribute*'s summary token (`IrAttribute`), while
`T0011` is reported at the *default value*'s (`IrAttributeDefinition`), which
is why a bad default points at the value and not at the port. And the
`ErrorCode::IncompatibleOutputDefinitionType` branch in
`IrAttributeDefinition::_checkTypes` is dead code -- the earlier branch already
covers every port direction that can own a default -- so the port has no
equivalent, with a comment saying so.

The distilled unit tests in `types.rs` are worth reading for one reason: the
`literal_int` definition is load-bearing. An integer literal only has a channel
type to compare once `public node literal_int => __piranha__literal_int { alias
output __out [int_channel]; }` exists, so the smallest script that reproduces
`T0010`/`T0011` is not the obvious one. Finding that was faster by deleting
line ranges from the fixture than by reading it.

## The builtin port table is recorded, not transcribed

`validateBuiltinMappings` has a second half: every port a `=>` definition
declares has to match a port on the builtin's *reference node* -- undefined
inputs and outputs (`V0080`/`V0081`), a `modify` port where the reference does
not modify (`V0090`), a `toggle` where it does not toggle (`V0100`), an `alias`
output where that name is not the reference's primary output (`V0110`). The
report moves to `MATCH 88, DIFF 6`:

| | + type check | + builtin port checks |
|---|---|---|
| MATCH | 87 | **88** |
| DIFF | 7 | **6** |
| `V0080`/`V0081` | 1 / 1 | 0 / 0 |

That reference node is a real `Node` instance built by
`Rule::generateReference`, and its ports are added by C++ constructors in the
`.cpp` files -- `PipeNode<T>` names its inputs after the wrapped output's
connection points, `OperationNode` numbers them, `NodeContainer` is the only
thing that ever sets `modifiesInput`/`enableInput`. So unlike the literal,
operator and conversion tables, this one cannot be read off the sources. It is
*run* instead: `script_reference --builtin-ports` calls the same
`getInputPortInfo`/`getOutputPortInfo` the validator calls, for every registered
name, and `record_builtin_ports.sh` records the answers into
`builtin_ports.txt`. `extract_language_rules.py` folds that file into
`language_rules.rs`, so there is still exactly one generated table and one
`--check`.

Two things the shape of that file has to get right:

1. **"No ports" is not "no such builtin."** The first recording omitted
   builtins whose reference node had no ports at all, and the port silently
   stopped reporting `V0080` for them -- `__engine_sim__float` is a bare
   `ChannelNode`, so every port a `=>` definition declares on it is an error in
   the original. The dump now emits a `builtin,<name>` row for every registered
   name, and `every_registered_builtin_has_a_port_table` fails if one goes
   missing.
2. **The oracle could not point `V0090`/`V0100` at the right line.** The IR
   dump carried the direction keyword's spelling but not its position, so the
   injected `IrAttributeDefinition` got a synthesized direction token on line 1.
   Codes still matched and the recorded fixtures compare codes rather than
   lines, so it was left documented -- until the same synthesis turned out to
   move a definition's *summary* token, and with it the `R0080` line in
   `infinite_loop_test_1`. The dump now carries the direction as a token and the
   artifact is gone (see the last section).

Three negative fixtures pin the four codes (`builtin_port_not_on_reference`,
`builtin_alias_mismatch`, `builtin_modify_mismatch`), which took the error gate
to 20 fixtures.

## The reference phase's second entry point, and one visibility bug

Six diffs were left, and five of them were the same missing piece: the phase has
*two* entry points, and only one was ported.

`IrCompilationUnit::_checkInstantiation` walks the unit's components against one
fresh main context. That is the entry point the port had. The other is
`IrNode::_checkInstantiation`: every structure is visited once
(`m_instantiationChecked`), components first, and a node then checks its
*definition* against a **main child of a fresh non-main root, focused on the
instance**. That is what makes a definition body's names resolve through the
instance -- and it is where `R0060` comes from, because the same expression that
dead-ends in the definition's own empty context resolves in an instantiation
context and is then missing its member. Alongside it, `IrNode::checkReferences`
overrides the base walk: it refuses to descend into a context that already holds
the node (`findContext`) and gives the definition a child context, while the
node's own *attributes* are still checked against the input context.

Shrinking `deep_error_isolated` showed the difference in one run:

```
node confusion {
    input some_input;
    output test: some_input.a;   // R0060 here, but only through an instance
}
confusion test(5)
```

With both entry points in place the report went `MATCH 88 -> 93`, and the only
diff left was the `R0080` link. Getting there surfaced a **pre-existing**
visibility bug worth recording: `IrAttribute`'s constructors all call
`setVisibility(IrVisibility::Public)`, so an instance's attributes are
externally accessible (`myNode.A`), and only body *nodes* keep the private
default that `R0061` reports. The port left attributes at the
`IrParserStructure` default (private), which cost one error in
`general_syntax_test_37` (it only showed up once definitions were checked per
instantiation) and the extra `R0061` in `resolution_errors_1`; both disappeared
when the attribute was lowered as public.

The last diff was not a port bug at all. `infinite_loop_test_1` reports `R0080`
at the *summary token* of the link it returns to, and one of those links is an
`IrAttributeDefinition`. A definition's summary token combines its registered
tokens, one of which is the **direction keyword** -- and the IR dump carried the
direction's spelling but not its position, so the oracle synthesized it at line
1 (the same artifact noted under `V0090`/`V0100`). The dump now carries it as a
token (`(dir (tok "input" 1 <line>))`), which fixes the `R0080` line, the
`V0090`/`V0100` lines, and that whole class of "the port is right and the oracle
is wrong" reports.

That closes the language gate:

| | before | after |
|---|---|---|
| MATCH | 88 | **94** |
| DIFF | 6 | **0** |
| `R0060` missing | 5 | 0 |
| `R0061` extra | 1 | 0 |
| `R0080` on the wrong link | 1 | 0 |

Zero diffs is not "the port is finished": 19 fixtures still fail to parse, and
171 errors were still excluded at that point. The next two sections say why
neither was a gap in the IR pipeline, and the third records what it took to
retire the exclusion.

**The lesson this unit adds:** a phase with two entry points looks almost right
with one of them, because the second only changes *which context* a structure is
checked in. The missing part is invisible in the port and obvious in the
original's `resolveAll` -- so when a family of errors is missing, the first
question is now "which traversal am I not doing", not "which rule is wrong".

## The 19 fixtures that cannot be scored

`MATCH 94, DIFF 0` counts the fixtures the harness *can* compare. The rest used
to be one undifferentiated `PARSE 19` bucket, which read like a to-do list. It
is not: the runner now splits it, and both halves are unscorable for reasons
outside the port.

**15 fixtures whose expected behaviour is the original parser's error output.**
Every one of them is deliberately malformed, and the code that would report the
error is bison -- absent from this environment, which is why the reference build
takes its IR from the Rust dump in the first place. There is nothing to compare
against, so the port cannot be right or wrong here:

| fixture | what makes it unparseable |
|---|---|
| `general_syntax_test_13` | `output formatted: "…" + some_input` with no `;` |
| `general_syntax_test_19`, `_20` | `input data` with no `;` |
| `general_syntax_test_21`, `_22` | `input data: 10` with no `;` |
| `general_syntax_test_3` | `0test` is not a literal; a trailing comma in `print_header(),` |
| `general_syntax_test_4` | `input inputs;;` and another `0test` |
| `construction-tests/demo.mr` | a nested call left unclosed (`FrameAdder fa(`) |
| `construction-tests/test.mr`, `stress-testing/stress_test_1_isolated.mr` | `default output …` -- there is no `default` keyword in `specification.y` |
| `construction-tests/nested_conversions.1.mr` | `public literal float => …` and `public float >> int => …` -- stale syntax, neither `literal` nor a `>>` conversion rule exists |
| `full-error-testing/test_case_1`, `_2`, `_3` | `###` and `$` are unidentified tokens, which is the point of the test |
| `syntax_error.mr` | an unclosed `(` |

The gtests confirm this reading: `GeneralSyntaxTest_13` expects exactly one
`UnexpectedToken` on line 8 -- the line of the missing `;` -- and `_19`/`_21`
expect one on lines 7 and 17. The port rejects the same line, so it agrees with
the original as far as it can. What it cannot do is what bison does next:
`_19` and `_21` go on to expect `IncompatibleType`/`InvalidOperandTypes` errors
further down the file, which means the original *recovered* from the syntax
error and compiled the rest. A hand-written recursive-descent parser that stops
at the first error has no such list to produce, which is why these fixtures can
never be scored rather than merely being unscored today.

**4 fixtures whose import target is not in the repository.** `missing_dependency.mr`
imports `fake_file.mr` on purpose (its gtest expects a single `IO0010`), and the
three `single_node_*.mr` fixtures import `test.sdl.mr`, which is not in the
piranha submodule at all -- not a file this checkout is missing, a file upstream
never shipped. Both sides abort on a failed import: the port in `resolve::load_graph`, and
the oracle in its own `analyze`, which needs the imported unit to exist. So
reporting `IO0010` in the port would change nothing here.

Those 19 are the whole of the unscorable population. Everything the harness can
parse and dump is now scored, with nothing excluded -- see the next section for
the 171 errors that used to be.

## The unspecified-token errors

171 of the reference's errors sat on an unspecified token (`line 2147483647`) and
were excluded from the gate, on the reading that they were an artefact of the
oracle rather than something the port could be wrong about. That reading was
wrong in a subtle way: the errors were faithful, and the port simply did not
produce them.

The shrink is two lines. Against the reference:

```mr
private node A {
    input x: 1;
}

A a(foo: false)
```

```
reference: R0010,1 R0010,2147483647
port:      R0010,1
```

`R0010,1` is the undefined type on line 1. The other is a literal.

**What the original does.** `IrValueLiteral::_expand` looks the literal's builtin
name up in the rules, looks that name up in the unit
(`resolveBuiltinNodeDefinition`), and has a `nullptr` branch for each step --
both of them *comment-only*: `// TODO: raise error, literal type not defined or
not available`. It then carries on and builds the expansion anyway, with
`setDefinition(nodeDefinition)` and that `nodeDefinition` still null. The node is
an `IrLiteralNode`, and `IrLiteralNode` never sets `m_type` -- `setName` assigns
the node's *name*, not its type. So the expansion's own `resolveDefinitions()`
calls `resolveNodeDefinition()` with the empty type name, finds nothing, and
reports `UndefinedNodeType` against `getTypeToken()`: a default-constructed
`IrTokenInfo`, whose `lineStart` is `INT_MAX`. `es/engine_sim.mr` never shows
this because its stdlib defines every `__engine_sim__literal_*` it uses.

**What the port did.** `expand_literal` mirrored the two comments as *early
returns* -- `let Some(definition) = definition else { return; }` -- which reads
like a faithful transcription of a TODO and behaves like a rule the original does
not have. Removing the return, letting the node carry `definition: None` and an
unspecified type token, and calling `resolve_definitions` on it reproduces every
one of the 171 errors. The counts came out right on the first run: expansion is
already keyed by `(structure, context)`, so a literal inside a definition used
twice expands twice, exactly as upstream.

**One line-number detail.** `error_dump` printed `error.location.line`, and a
synthesised token's line is `0` in the port, so these errors would have come out
as `R0010,0` against the reference's `R0010,2147483647`. `dump.rs` already
translated the sentinel for `--lines` dumps; that translation is now
`Token::line_start()`, and the three places that show a line -- `--lines` dumps,
`--errors`, and `format_line` -- use it, which is also what `IrTokenInfo`'s
constructor says the default token is.

**Consequences of the change:**

| | before | after |
|---|---|---|
| gate | `MATCH 94, UNSPECIFIED 171`, `--strict` `DIFF 29` | `MATCH 94`, no exclusions |
| `--strict` | kept the excluded errors | removed -- it no longer means anything |
| negative fixtures | 20 | 21 (`literal_type_not_defined.mr`) |
| `run_script_parity.sh` | `PASS: 16263` | `PASS: 16263` (no change) |

The script-parity gate is the control: nothing in the engine's own scripts
changed, because they define their literal builtins. Only a fixture that leaves
its `__*__literal_*` definitions out gains errors.

One existing unit test had been passing for the wrong reason and had to change:
`types::tests::an_argument_that_converts_is_accepted` compares a string literal
against a string port in a fixture that declared only `literal_int`, so the
string literal had no definition and now reports `R0010` -- which is what the
reference reports for it. The fixture gained a `literal_string` definition, and
the test now reports only the type errors it is about.

## The node graph

Every gate so far stops at `resolveAll`. The phase after it is `build`:
`IrCompilationUnit::build` creates a root `IrContextTree`, walks the unit's node
list and calls `generateNode` on each, which is what turns the resolved IR into
`Node` objects wired together by `NodeOutput` pointers. That graph is what the
engine's bindings then execute, and it had no oracle.

`script_reference --graph <root.mr> <dump>` is that oracle. It runs the same
injection and `resolve`/`validate` stages as the other modes, then calls
`root->build(&program)` and dumps the result:

| record | fields |
|---|---|
| `G` | container id, name, parent id, node count, child count |
| `CI` | container input: container id, index, name, modifies, enable |
| `CO` | container output reference: container id, index, name, target node, target port, node output |
| `N` | node: container id, node id, name, builtin name, virtual, input count, output count |
| `O` | output port: node id, index, name, channel type, interface node |
| `I` | input port: node id, index, name, connection node, connection port, dependency node |

`-` is a null pointer. Node ids are assigned in traversal order — a container's
own nodes in insertion order, then its children depth first — so they describe
the graph rather than the allocator.

Two programs produce that text. `script_reference --graph` is the original;
`graph_dump` (`crates/engine-sim-script/examples`) is the port, which builds the
graph with `Arena::build_graph` and prints it through `graph::dump`. The dumper
lives in the crate rather than the example so the fixture test
(`tests/graph_tests.rs`) uses the same one.

`tools/parity/run_graph.py` checks the fixtures in `tools/parity/graph/`: the
recordings are made from the reference (`--record`, which also checks the port
against what it just recorded) and every run compares the **port** against them.
The fixtures are deliberately tiny, one idea each:

| fixture | what it pins |
|---|---|
| `literal_float.mr` | one builtin literal node, its name (`"1.5"`-style quoted text is the node's *name*), its `__out` port and its channel type |
| `node_identity.mr` | an instance with an input bound to a literal: where the literal's node lands, how the container's input and output reference are wired |
| `literal_in_container.mr` | a literal that belongs to the instance's own container instead of the root |
| `two_instances.mr` | the instance cache is keyed by (definition, context): two instances of one definition get two containers, each with its own literal |
| `builtin_channel_node.mr` | a definition that maps onto a channel class instantiates like any other builtin and registers no ports |
| `nested_body.mr` | a body that constructs a node: a container that holds nodes of its own, and a `CO` record with both a port and a node output |
| `two_inputs.mr` | positional and named arguments both connect; two literals in one container, in argument order |
| `modify_port.mr` | a `modify` port sets the container input's flag (`CI,...,1`) |

Three things the graphs say that the code does not:

* **Interface nodes are a vector-only affair.** `NodeOutput::newInterface`
  returns `nullptr` for every output class except `VectorOutput`, so
  `IrAttributeDefinition::_generateNode`'s "this must be an interface" branch
  produces nothing for scalars, and `getInterface()` is `-` in the `O` records.
  A port that reads that as "the node is missing" will go looking for a bug that
  is not there.
* **`addNode` is not "push onto this container".** `NodeContainer::addNode`
  starts with `if (getTopLevel()->findNode(node)) return;`, and a node generated
  for an instance's input is added to the *root* container. In `node_identity`
  the container `d` therefore holds no nodes at all while root holds both the
  literal and `d` itself.
* **A container's outputs are not its outputs.** They are
  `m_outputReferences` (`CO`), registered by `addContainerOutput` through a
  `pNodeInput` connection point; `m_outputs` stays empty.

Graph fixtures have to be clean: `build` dereferences the resolved definition, so
a script with errors has no graph to dump.

**The fixture set stops at plain ports on purpose.** An operator needs a node
class for the operation *and* IR definitions for its ports; `value: 2 * 3` under
test rules builds `int_multiply` + `int_to_float`, and the port generates the
conversion without the multiply under it. The probe is 15 records:

```text
G,0,root,-1,4,1
N,0,0,"2",__piranha__literal_int,0,0,1
...
N,1,4,,__piranha__int_to_float,0,1,1
I,4,0,__in,2,__out,-      # reference: the multiply's output
I,4,0,__in,-,-,-          # port: nothing
```

The cause is in `expansion.rs`, not in the builder: when an operand expands
through a conversion, the `IrInternalReference` it builds must point at the
operand *structure* (`IrBinaryOperator`), and the port's points at the resolved
fixed type instead, so generation walks into that type's interface node — which
`IrAttributeDefinition::expand` marked `setBuild(false)`, i.e. nothing. Fixing
that is what makes an operator fixture possible.

**`assets/main.mr` is not comparable yet, and the reason is M4.** Building it
needs `es_script`'s own node classes (`__engine_sim__set_engine`,
`__engine_sim__impulse_response`, the engine objects, ...), whose ports live in
that library, not in piranha. `Graph::missing_builtins` records every class the
factory answered `nullptr` for, `graph_dump` prints them, and
`tests/graph_tests.rs` fails a fixture that needs one — so an unported class
shows up as a missing node rather than as a shorter graph nobody notices. On
`assets/main.mr` that list is the four `set_*` actions; the reference's 6524
records are behind them.

**Status.** The port builds the same graph as the reference on all eight
fixtures, 67 records, byte for byte; `run_graph.py` and `tests/graph_tests.rs`
enforce it, and `run_script_parity.sh` runs the former as its `GRAPH` gate. What
is *not* ported yet is everything the graph is for: `initialize`, `evaluate` and
`NodeProgram::execute`, so the graph is only checked for its shape.
