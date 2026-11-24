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
the runner excludes unspecified-token errors by default, `--strict` keeps them.

`PARSE` 19 is the parser's own to-do list -- files the original accepts and this
parser rejects. They cannot use the oracle (no dump exists if the parse fails),
so they are compared against the gtest literals directly.

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

*(Expansion did land; the numbers moved to `MATCH 50` with 19 over-reported
`R0050`s. See "What is left" at the end of this document for the current state
and the next cause.)*

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

## What is left: the reference phase, and why it stays opt-in

`checkReferences` (`_checkInstantiation`) now finds every `R0050` the original
reports but **over-reports** `R0050` on 19 fixtures, so it stays behind
`SCRIPT_CHECK_REFERENCES=1`:

| | without | with |
|---|---|---|
| MATCH | 69 | 57 |
| `R0050` missing | 18 | 1 |
| `R0060` missing | 19 | 5 |
| `R0080` missing | 3 | 1 |

Sharing the expansions between phases cut the over-reporting roughly in half
(`R0060` 14 -> 6, `R0080` 3 -> 1). One more cause fell out of a five-line probe:
**`auto node X` also instantiates a node named `X`**, before registering the
definition (`immediate_node_definition: AUTO specific_node_definition`), and
`IrCompilationUnit::resolveLocalName` finds *nodes*. Without the name, a
reference to `X` from inside `X`'s own body resolved in the original and not in
the port (`general_syntax_test_30/32/33`); with it, 44 -> 37.

What is left is a resolution question, not a reporting one: a label whose
reference should succeed through the scope chain still fails in the port, which
is why `std::vector(vector.w, vector.z, ...)` yields four invented `R0050`s in
`general_syntax_test_8.mr`, and why some `R0050`s come out *repeated* (three
copies on one line) -- that repetition is a clue worth following, because the
original visits each structure once. The next piece of work is the same shape as
this one: shrink a fixture to the smallest script that resolves in the original
and not in the port, then fix the model.

The lesson that keeps repeating: the failing fixture mixes several questions at
once. Five-line probes answer them one at a time in minutes, and every one of the
causes above was found that way -- not by reading the fixtures that failed.
