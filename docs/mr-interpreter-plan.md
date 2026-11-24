# `.mr` interpreter stage — aspects, options, contest, plan

Status: planning note for the next stage of the Rust port. Everything below is
measured from this repository, not assumed.

---

## 0. What is actually blocked

The `.mr` stage must turn a script (`assets/main.mr` → `es/engine_sim.mr` →
engine files) into the parameters the already-bit-exact core consumes. The only
authority for those numbers is the C++ stack: `piranha` (interpreter) +
`scripting/` (engine bindings) + `es/` (the standard library written in `.mr`).

That stack cannot currently be built here:

| Fact | Evidence |
|---|---|
| The interpreter front end exists only as flex/bison input | `piranha/flex-bison/{scanner.l,specification.y}` (170 + 525 lines) |
| No generated sources are checked in | `piranha/include/scanner.h` includes `<parser.auto.h>`, which does not exist |
| No generator is installed | `flex`, `bison`, `yacc`, `m4`, `cmake` all absent; `apt` has no candidate lists |
| PyPI / npm cannot supply them | `pypi: bison`, `pypi: flex`, `npm: bison`, `npm: flex` are unrelated JS/Python packages |
| crates.io is not reachable | so "just use a Rust parser generator crate" needs the user to vendor it |
| piranha links Boost | `find_package(Boost ... filesystem)`; `/usr/include/boost` absent (only `path.cpp`, 81 lines, uses it) |
| The rest of piranha is plain C++ | `src/*.cpp` 5 515 lines, `include/*.h` 4 444 lines; the generated parser's *types* are referenced by 10 translation units, but it is only ever *invoked* in `IrCompilationUnit::parseHelper` |
| The engine bindings are plain C++ too | `scripting/` 2 539 header + 582 source lines, 21 `*_node.h` headers plus `actions.h`, **79 distinct `__engine_sim__*` builtins**, `es/` defines 148 script-level nodes |
| Corpus | `es/` 18 `.mr` files / 1 777 lines + `assets/` 40 `.mr` files / 9 344 lines + recorded `.wav` impulse responses |

So "bit-exact `.mr`" needs a *reference run of the C++ scripting stack*, and the
only missing piece is the parser. Everything else is buildable today once
`parser.auto.h` exists as a declaration (see A4).

---

## 1. Aspect A — where does the reference oracle come from?

| Option | What it is | Fidelity | Cost to me | Cost to user | Fragility |
|---|---|---|---|---|---|
| **A1** | user supplies `bison`, `m4`, `flex` (static Linux binaries) | 100 % | low | one download | glibc/arch must match; `/tmp` is wiped every session, so they must live in the repo or be re-supplied |
| **A2** | user runs the two generator commands once and supplies the 4 generated files (`parser.auto.{cpp,h}`, `scanner.auto.{cpp,h}`) | 100 % | low | one command + 4 files | none — plain C++, committed once, reproducible forever |
| **A3** | I write an LALR(1) table generator (Python, or via `ply`) plus a bison-skeleton driver, and run the upstream *semantic actions* on my tables | high | **high** (days) | none | my generator's conflict resolution must match bison's; error recovery differs |
| **A4** | no generated parser at all: I write the ~150-line `parser.auto.h` **declaration** stub (token enum from `%token`, `semantic_type` from `%type`, `location_type`), build *all* of piranha + `es_script` unmodified, and have my Rust parser emit its IR into the reference through a JSON dump | everything except the parser | medium (1–2 days) | none | duplicates the ~500 lines of bison *semantic actions* in the dump→IR bridge; parser itself is unverified |
| **A5** | no reference build: trust the TRX520 fixture + upstream unit-test expectations | partial (1 of 40 assets) | low | none | 39 assets have no absolute oracle; silent misparse possible |
| **A6** | find a fork/copy of piranha that committed the generated sources | 100 % | low *if it exists* | none | lottery; would also need to match this submodule commit |

### Contest

* **A3 loses on cost/risk ratio.** It buys the same thing A2 buys for a fraction
  of the effort, and it introduces a *second* implementation that itself needs
  validating. Only worth revisiting if the user cannot supply anything at all
  *and* A4 proves insufficient.
* **A5 alone is not acceptable** for a project whose bar is "bit-exact against
  the C++", because 39 of the 40 assets would be unverified. It stays as an
  *interim gate* only.
* **A4 is much cheaper than it first looks**, because `parser.auto.h` is a
  *declaration-only* surface: the token enum is the 41 `%token` declarations (43 named tokens),
  the semantic value type is the union of the `%type` declarations (9 pointer
  types + `IrTokenInfo_string` + `IrTokenInfoSet<std::string, 2>`), and
  `location_type` is already fixed by `%define api.location.type
  {piranha::IrTokenInfo}` — a hand-written class, nothing to invent.
  With that header, `compiler.cpp`, `ir_compilation_unit.cpp`, the whole node
  graph, `es_script` and the `es/` library all compile and run unmodified — the
  only thing missing is `Parser::parse()`'s body, which A4 does not need.
* **A2 and A4 are complementary, not alternatives.** A4 covers IR resolution,
  the type system, the node graph, `es_script` and `EngineContext`; A2 adds the
  parser. Running *both* against the Rust implementation gives two independent
  comparisons (and a much better failure signal: a mismatch that appears in A2
  but not A4 is a parser bug; one that appears in both is a semantic bug).

### Refined position

1. **Build A4 first.** It needs nothing from anyone, and it converts "blocked"
   into "95 % of the stage is verifiable now".
2. **Ask for A2 in parallel** (4 files, one command). When it lands, wire the
   generated parser into a second reference binary and re-run the same harness.
3. Keep A1 as an equivalent substitute if generating is inconvenient.
4. Reject A3 and A6; keep A5 only as the interim gate for milestone M1.

---

## 2. Aspect B — how the Rust parser gets written

| Option | Contest |
|---|---|
| **B1** hand-written recursive descent mirroring `specification.y` production by production | The grammar is *precedence-structured* (`mul_exp` / `add_exp` / `unary_exp` / `primary_exp`) with **no `%left`/`%right`/`%prec`** declarations, so RD maps 1:1 with no precedence table to get wrong. Only `error`-recovery productions are dropped — they affect only invalid input. **Wins.** |
| **B2** table-driven parser from generated LALR tables | Buys nothing: same language, more machinery, and I would still have to hand-write the action table. Loses. |
| **B3** `lalrpop`/`chumsky`/`nom` | Adds a build-time dependency to a crate that is deliberately dependency-free, and the grammar dialect differs from bison in ways that are easy to get subtly wrong. Loses. |

**Decision: B1**, with the scanner ported rule for rule (30 rules; no start
conditions, no `REJECT`, no trailing context; `//` and `/* */` handled
manually; numbers are `D+`, `0x[H]+`, `D+.D+` — *no exponent, no leading dot*;
strings are raw, no escape processing; anything else is `UNRECOGNIZED`).

Verification of the parser is a *differential* test (needs A2): both parsers run
over every `.mr` file in `es/` and `assets/`, plus a generated corpus of
mutations (truncations, token substitutions), and must agree on accept/reject
and on the IR dump.

---

## 3. Aspect C — how much of piranha's runtime to port

| Option | Contest |
|---|---|
| **C1** faithful port of the whole pipeline: parse → `IrCompilationUnit` (attribute definitions, type resolution, visibility, inlining, `=>` shadowing) → `build(&program)` (instantiate node graph) → `NodeProgram::initialize/execute` (pull-based evaluation through channels) → `es_script` nodes fill `EngineContext` | The only option that runs the *unmodified* `es/` library, whose types and operators are themselves node definitions (`inline node float { input __in [::float_channel]: 0.0; ... }`). Wins. |
| **C2** faithful front end, simplified eager evaluator instead of the node graph | Saves maybe 1 200 lines but loses the reference's evaluation order. That order is load-bearing here: `add_rod_journal`, `add_ignition_module`, bank/cylinder registration all mutate `EngineContext` in evaluation order, and we already know from the core port that *registration order changes the physics* (constraint order bug, PR #1). Only viable if proven order-equivalent, at which point it is C1 with extra risk. Loses. |
| **C3** bespoke `.mr` reader that builds `EngineContext` directly | Reimplements the standard library natively instead of interpreting it; guaranteed drift. Loses. |

**Decision: C1**, with the port *sized by measurement*: instrument the reference
build (A2/A4) to log which node classes and which `es_script` bindings are
actually instantiated while compiling all 40 assets, and port that set. The 79
`__engine_sim__*` builtins are the upper bound.

---

## 4. Aspect D — type system and conversions

`es/types/atomic_types.mr`, `conversions.mr` and `operations.mr` define the
type system *in the language itself*, and `es_script/language_rules.cpp`
registers the conversion nodes. Options: port piranha's resolution +
`ChannelType`/`FundamentalType` machinery exactly (D1, wins) versus hardcode the
conversions seen in `es/` (D2, breaks the moment the library changes).

Add a dedicated dump to the reference: for every attribute assignment, the
*resolved* target type and the converted value. Comparing that stream localises
coercion bugs immediately instead of via a wrong engine parameter months later.

---

## 5. Aspect E — evaluation order and side effects

| Option | Contest |
|---|---|
| **E2** instrument the reference (`EngineContext` mutation log: `seq, action, args`) and compare the sequence with Rust | Localises ordering bugs to a single statement. Cheap: the same patch-a-copy trick used for `combustion_chamber.cpp`. Wins as the *diagnostic*. |
| **E1** rely on end-to-end trace equality | Works (any ordering error shows up as diverging dynamics) but is useless for *finding* the cause. Wins as the *gate*, not as the tool. |

**Decision: build E2, gate on E1.**

---

## 6. Aspect F — environment and persistence

* `/tmp` is wiped between sessions; anything the reference needs must either be
  **committed to the repo** or regenerated by a script.
* **F1 (wins):** commit the A2 generated sources under
  `rust/tools/parity/generated/`, with a `README` recording the exact commands
  and bison/flex versions, and a `stage_reference.sh` step that compiles them.
  ~200–400 KB of plain C++, no binaries, licence-clean (bison-generated code has
  a special exception; flex output is BSD-ish).
* **F2:** vendor binaries (bloat, licence, arch-specific) — avoid.
* **F3:** re-fetch each session — impossible without the hosting host being
  reachable.

Also to record in the reference build (no arithmetic impact): Boost →
`std::filesystem` in `path.cpp` (81 lines, 9 call sites), exactly like the
existing `reference.patch` concessions.

---

## 7. Aspect G — what "verified" means for this stage

| Option | Contest |
|---|---|
| **G3** node-graph/mutation trace | diagnostics (see E2) |
| **G1** dump the resulting `EngineContext` (all parameters) and compare field by field | the natural checkpoint, localises to a parameter |
| **G2** build a `Simulator` *from the script* and run the existing core harness (24 fields + synth input + audio) against the C++ doing the same | the gate: it is end-to-end, it reuses machinery that is already bit-exact, and it retires the hand-written fixture in favour of the real pipeline |

**Decision: G3 → G1 → G2, gate on G2 for every engine asset.**

---

## 8. Plan

| # | Milestone | Gate | Notes |
|---|---|---|---|
| M0 | Reference scaffolding: `parser.auto.h` stub (A4), Boost→`std::filesystem`, build `piranha` + `es_script` + a driver that loads a JSON IR dump, dumps `EngineContext`, and logs mutations | the driver reproduces the TRX520 fixture from a dump the *Rust* parser produces | no user input needed |
| M1 | Rust: scanner + parser + IR structures (`engine-sim-script` crate, dependency-free) | all 18 `es/` + 40 `assets/` files parse clean; accept/reject differential vs A2 | ~1 500 lines |
| M2 | Rust: IR resolution + validation (types, defaults, visibility, aliasing, inlining, `=>`) | per-attribute type/value dump equals reference for all assets | ~1 500 lines; split into M2a (substrate), M2b (resolution), M2c (validation + type dump) — see §8.3 |
| M3 | Rust: node graph + execution (`Node`, `NodeOutput`, `Channel`, `NodeProgram`, allocator) | mutation-log sequence equals reference | ~1 200 lines |
| M4 | Rust: `es_script` bindings + `EngineContext` → `Engine` construction in `engine-sim-core` | G2 (full trace + audio) for **all** engine assets, fixture retired | ~2 500 lines, the largest chunk |
| M5 | WAV impulse responses (`es/sound-library/*.wav`) + `impulse_response_library` | decoded samples bitwise equal; audio matches with the real IR | ~300 lines |

Each milestone is independently verifiable, and M0 is what unblocks everything.

### 8.1 Status

| # | State | What actually shipped |
|---|---|---|
| M0 | ✅ | `tools/parity/stage_script_reference.sh` stages and compiles `piranha` + `es_script` (`/tmp/refscript/librefscript.a`) with the parser removed; `tools/parity/script_reference.cpp` injects the IR and drives the real pipeline |
| M1 | ✅ | `crates/engine-sim-script` (`lexer`, `ir`, `parser`, `dump`, `resolve`), dependency-free, 27 tests |
| M2a | ✅ | `lower` (parse tree → arena), `channel_type`, `error`; every unit of both entry points lowers and raises back field-for-field; 95 tests |
| M2b | ✅ | done: `resolveName`/`resolveLocalName` (scope walk, unit symbols, node attributes, definition ports/body, definition-local-only), `countSymbolIncidence`, `IrContextTree` (`context.rs`: chains, `isEqual`, `isOutside`, `hasParent`, `findContext`, `getMain`/`getRoot`, `resolveDefinition`). (the reference chain itself is still only needed by `_checkReferences`/`expand`, which is M2c) |
| M2c | ◐ | `checkCircularDefinitions` ported (E0020, with the instantiation chain) and gated by 2 fixtures; `_validate` ported (unit/node/definition checks, `validateBuiltinMappings`' `V0070`) and the **error gate is green**: 20 negative fixtures in `tools/parity/negative/` reproduce the reference's exact code multiset (`tests/error_tests.rs`). The `--resolved` gate is **green**: `tests/resolved_tests.rs` requires 660/660 (`es/engine_sim.mr`) and 701/701 (`assets/main.mr`) rows identical to `script_reference --resolved`. Covered: `_resolveDefinitions` (definition lookup with imports/visibility, port types, alias types, attribute→port), `resolveNodeDefinition`, the builtin channel-type table, `expand` (`expansion.rs`, with `m_expansions` on the arena), the reference chain (`reference.rs`, `_checkInstantiation`), and the type check (`types.rs`, `_checkTypes`). `validateBuiltinMappings` is complete (`V0070`-`V0110`, against the builtin reference nodes' ports recorded in `tools/parity/builtin_ports.txt`), gated by 20 negative fixtures. The language gate is at `MATCH 88, DIFF 6` of 258 fixtures. Still open: the residual `R0060`/`R0061`/`R0080` diffs |
| M3–M5 | ⬜ | unchanged |

Two deviations from the plan, both forced by the missing toolchain:

* **A4 became "remove the parser" rather than "stub `parser.auto.h`".** Only
  `Parser::{location_type, semantic_type}` and `m_parser->parse()` were reachable
  from the rest of piranha, so `parseHelper` is stubbed and every unit is filled
  by IR injection instead. The oracle therefore covers everything *downstream*
  of the parser — resolution, validation, node generation, execution, the
  `scripting/` bindings — which is the part the port has to match.
* **The M0 gate is now the IR round trip, not "reproduce the TRX520 fixture".**
  The TRX520 script (`assets/engines/atg-video-1/01_honda_trx520.mr`) is in the
  old language and cannot be parsed by the current grammar, and a script-built
  engine can only be compared with the hand-built Rust fixture once the
  bindings exist (M4). What M0 *can* prove, and does, is that the original
  pipeline accepts the IR the Rust parser produces (`--check`: resolve +
  validate clean for all 58 scripts) and that it builds and runs a real engine
  from it (`--run assets/main.mr`: 16 263 engine and trace checkpoints,
  recorded in `rust/crates/engine-sim-script/fixtures/main_engine.trace`).
  That recorded trace is the G2 target M4 has to hit.

### 8.3 M2's shape: why the IR is lowered into an arena

M2 is not a leaf-to-root walk over the parse tree. piranha resolves *in place*
and every phase is driven by state that a pure tree cannot express:

* mutable per-structure fields (`IrNode::m_definition`,
  `IrAttributeDefinition::m_typeDefinition`, `IrNode::m_builtinInfo`);
* identity comparison — circular-reference detection pushes
  `(structure, context)` pairs onto a set, and `IrContextTree` nodes are
  compared by pointer;
* memoisation flags (`m_definitionsResolved`, `m_expanded`, `m_typesChecked`,
  `m_instantiationChecked`, `m_validated`), so a shared definition is resolved
  once and reused;
* a traversal order given by `registerComponent`, which is *not* the parse
  order (an `auto node` instance is registered before its definition; a
  definition registers its port list before its body).

The three options considered:

* **A — mutate the parse tree in place.** Impossible for the identity and
  memoisation requirements without `Rc<RefCell<_>>` everywhere, which is the
  same thing as an arena with worse ergonomics.
* **B — lower into a flat arena of `StructureId`s and work there.** Every
  structure keeps the `IrParserStructure` fields (parent, scope parent, logical
  parent, summary token, visibility, stage flags) plus a `Kind` payload whose
  component list reproduces `registerComponent` order exactly.
* **C — rebuild resolution on immutable data with side tables.** Least faithful:
  the reference chain's early-out behaviour (an `IrAttributeDefinition` with a
  fixed type stops a chain) and the instantiation-context propagation are
  emergent properties of the mutable graph, not of a declarative spec, so a
  rewrite would drift where the gate cannot see it.

**B chosen.** The parse tree stays untouched — it is what the M1 oracle bridge
and the dump consume, and it is already verified — and `Arena::to_unit` raises
the arena back so the lowering itself is checkable: `tests/lower_tests.rs`
lowers all 18 standard-library units and all 21 asset units and asserts
`to_unit(lower(unit)) == unit` field for field (tokens and their source
locations included). Measured sizes: 253 nodes / 653 attributes / 641 ports
(`es/engine_sim.mr`), 409 / 1 026 / 677 (`assets/main.mr`).

The reference dump for M2c already exists: `script_reference --resolved` prints
the original's own resolution output — per port, the direction, alias flag, the
node definition the declared type resolved to and the channel type it produced;
per unit-level attribute, the port it resolved to (660 rows for
`es/engine_sim.mr`, 701 for `assets/main.mr`, recorded into
`crates/engine-sim-script/fixtures/*.resolved` by `RECORD_RESOLVED=1`). Nothing
compares the Rust side against it yet, so it is a recorded target, not a gate.

The measured round-trip is the gate for the substrate only. M2b/M2c still owe
the differential gate: a per-attribute dump of the *resolved* type and value,
compared against `script_reference` running the real `resolve` + `validate`
over the same units.

### 8.2 The IR dump

`engine-sim-script` writes the IR as S-expressions, one `(unit "path" ...)` per
file, with imports, then definitions, then nodes — the three lists
`IrCompilationUnit` actually keeps, in that order. `script_reference --ir` reads
a dump back into piranha objects and re-emits it, so `compare_ir.py` compares
*two independently produced* trees rather than one text against itself.

Token flags: only `specified` is carried. `IrTokenInfo::valid` only selects the
error code in `Parser::error`, `empty` is never read (`grep -r "\.empty\b"`),
and a label's own token flags are not recoverable from `IrValueLabel`, so the
dump defines labels as always specified — which is exactly what the scanner
produces for them.

Two places where the original relies on the parser actions rather than on
constructors, and the injection has to mirror them:

* `IrAttributeDefinition`'s constructor leaves `m_defaultValue` and `m_isAlias`
  uninitialised, so `setDefaultValue(nullptr)` and `setAlias(false)` must be
  called even when the script wrote neither.
* `IrNode::setThis` does not store a "this" pointer: it appends an attribute
  named `this` to the node's attribute list. `data_access '.' node` is modelled
  the same way in the Rust IR.

### What would come from the user (only for A2 — M0 does not need it)

In a tree with `flex` and `bison` (3.2+), from the submodule root:

```sh
# bison: specification.y declares %defines, api.namespace{piranha}, api.parser.class{Parser}
bison --defines=parser.auto.h -o parser.auto.cpp flex-bison/specification.y

# flex: scanner.l uses %option yyclass="piranha::Scanner"
flex --header-file=scanner.auto.h -o scanner.auto.cpp flex-bison/scanner.l
```

Send `parser.auto.{cpp,h}` and `scanner.auto.{cpp,h}` (plus `bison --version`
and `flex --version`). Equivalent alternative: static `bison`, `m4`, `flex`
binaries (Debian 12 / glibc 2.36 here — a static build is safest).

If neither is possible, A4 alone still verifies everything except the parser,
and B1's parser is audited production-by-production against
`specification.y` with the corpus as the acceptance test.
