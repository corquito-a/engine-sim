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
| M6 | Rust: `engine-sim-cli` -- run a script's engine headlessly, write the audio and the telemetry | a driven run matches the reference for 2 s of combustion (`DRIVEN`); the binary's own contract is tested | ~400 lines |

Each milestone is independently verifiable, and M0 is what unblocks everything.

### 8.1 Status

| # | State | What actually shipped |
|---|---|---|
| M0 | ✅ | `tools/parity/stage_script_reference.sh` stages and compiles `piranha` + `es_script` (`/tmp/refscript/librefscript.a`) with the parser removed; `tools/parity/script_reference.cpp` injects the IR and drives the real pipeline |
| M1 | ✅ | `crates/engine-sim-script` (`lexer`, `ir`, `parser`, `dump`, `resolve`), dependency-free, 27 tests |
| M2a | ✅ | `lower` (parse tree → arena), `channel_type`, `error`; every unit of both entry points lowers and raises back field-for-field; 95 tests |
| M2b | ✅ | done: `resolveName`/`resolveLocalName` (scope walk, unit symbols, node attributes, definition ports/body, definition-local-only), `countSymbolIncidence`, `IrContextTree` (`context.rs`: chains, `isEqual`, `isOutside`, `hasParent`, `findContext`, `getMain`/`getRoot`, `resolveDefinition`). (the reference chain itself is still only needed by `_checkReferences`/`expand`, which is M2c) |
| M2c | ◐ | `checkCircularDefinitions` ported (E0020, with the instantiation chain) and gated by 2 fixtures; `_validate` ported (unit/node/definition checks, `validateBuiltinMappings`' `V0070`) and the **error gate is green**: 21 negative fixtures in `tools/parity/negative/` reproduce the reference's exact code multiset (`tests/error_tests.rs`). The `--resolved` gate is **green**: `tests/resolved_tests.rs` requires 660/660 (`es/engine_sim.mr`) and 701/701 (`assets/main.mr`) rows identical to `script_reference --resolved`. Covered: `_resolveDefinitions` (definition lookup with imports/visibility, port types, alias types, attribute→port), `resolveNodeDefinition`, the builtin channel-type table, `expand` (`expansion.rs`, with `m_expansions` on the arena), the reference chain (`reference.rs`, `_checkInstantiation`), and the type check (`types.rs`, `_checkTypes`). `validateBuiltinMappings` is complete (`V0070`-`V0110`, against the builtin reference nodes' ports recorded in `tools/parity/builtin_ports.txt`), gated by 21 negative fixtures. The language gate is at **`MATCH 94, DIFF 0` with nothing excluded**: the 171 unspecified-token errors (`R0010` from `IrValueLiteral::_expand` when a fixture does not define its literal builtins) are reproduced and scored, and 19 fixtures are unscorable (15 malformed, so their oracle is the bison parser this environment does not have; 4 import a file the repository does not contain). Both `checkInstantiation` entry points and `IrNode::checkReferences` are ported. The 19 unparsed fixtures are fully triaged and none is a port defect (15 are bison-reported syntax-error fixtures, 4 import a file the repository does not contain), so both the parser and the error gate are closed |
| M3a | ◐ | **the oracle half**: `script_reference --graph` builds the node graph (`IrCompilationUnit::build`) and dumps it -- containers, nodes, ports, connections, container inputs and output references -- and `tools/parity/run_graph.py` records and checks the graph fixtures in `tools/parity/graph/` (3 scripts, 17 records). The Rust node graph itself (`Node`, `NodeOutput`, `NodeProgram`) is **not** ported yet: `build` is what it has to reproduce, and `run_graph.py` becomes the gate for it unchanged |
| M3b | ✅ | **the builder**: `graph.rs` ports `IrParserStructure::generateNode`/`generateNodeOutput`/`generateAliasNode`/`generateAliasOutput`/`generateOutput`/`connectInput`, the `.` special case of `IrBinaryOperator::_generateNode(Output)`, and the two allocators (`GraphNode`, `Port`), so `Arena::build_graph` turns resolved IR into the same containers, nodes and connections as `IrCompilationUnit::build`. `LanguageRules::generateBuiltinType` is a table of `ClassSpec`s keyed by class name — piranha's own (literals, channels, operations, negations, conversions, `NoOpNode`'s output reference, `VectorSplitNode`'s interface input) and `es_script`'s 35 (`set_*` actions, the engine objects, `impulse_response`, ...) — so it answers for `__piranha__*` and `__engine_sim__*` alike, with `NodeClass::Builtin` standing in for behaviour until M4. `graph::dump` emits the reference's record format and `examples/graph_dump` is the port's half of `run_graph.py`: **11 fixtures, 119 records**, plus every `.mr` under `assets/` — **40 scripts, 20 008 records** (`main.mr` alone is 6524), all byte-identical; the scripts are compared live rather than recorded, and `tests/graph_tests.rs` holds the fixture line without a C++ build. Details worth keeping: `addNode`'s dedupe starts at the *top level*, not the target container; a container's outputs are `m_outputReferences`, not `m_outputs`; interface nodes are vector-only, so `-` on a scalar port is correct; a factory-built node's `primaryOutput` is what `generateAliasOutput` follows, so a class without one is a node with no way back to its value. Operators (`2 * 3 + 1`, `-x`, `units().mass`) cost four more: `_expand`'s output branch must keep `build` (only the dead end clears it); `_generateNodeOutput` has interface branches of its own, not just `_generateNode`'s `generateInterface` step; `generateAliasOutput`'s last resort is `getInterfaceInput()`, a slot only `VectorSplitNode` sets, so answering with the first input's connection invents 21 resolutions in `main.mr`; and a literal's name is `%g`-formatted, not shortest-round-trip |
| M3c | ◐ | **both gate-able halves done**. *Values*: `values.rs` is `full_compute` as a read-only walk — literals (their injected data, not the `%g` name), the four arithmetic families, negations, the conversions (`string_to_float` is `std::stringstream >>`, so a prefix and a `0.0` default), `string_add`, `NoOpNode`'s output reference, and `es_script`'s `k_28inH2O`/`k_carb` (`GasSystem`, via `engine-sim-core`); keyed on the class name after the rules prefix. *Program*: `program.rs` is `NodeProgram::initialize` plus `Node::evaluate` — top level then each container's children, early out when evaluated/evaluating, inputs first, then `_evaluate`, then output references — with the three classes whose `_evaluate` is observable (`console_out` writes, `throw_runtime_error` fails the run, `int_probe` computes and drops). Gated by `script_reference --values`/`--execute`: **3 value fixtures + 7 609 port values** across the 40 scripts of `assets/`, and **2 execute fixtures** comparing both the text written and whether the run failed, all byte-identical; `tests/value_tests.rs` and `tests/program_tests.rs` hold the same gate without a C++ build. Findings: the value layer is pull-based (nothing caches, the builtin `_evaluate`s are empty), and `+2.5` (`NoOpNode` feeding a container input) makes the original **segfault** — a node answering its own alias walk — so `build_graph` stops at the repeat (`Graph::recursion_exhausted`) instead. **Next**: M4, the `es_script` classes themselves; `checkEnabled`, `console_in`, `optimize`/`markDeadNodes` and `free` stay unported until something needs them |
| M4 | ✅ | **the engine, built from the script and simulated by the port**. `build.rs` is `EngineNode::buildEngine` and every `generate` under it: the counts, the fuel, the mean-piston-speed curve, the crankshafts and their rod-journal indices, the banks with their pistons and rods (a slave rod taking its master's crankshaft), the heads with their camshafts, lobe centrelines and port-flow curves, the intakes, the exhausts, the ignition module's posts resolved through the bank cylinder indices, and the chambers. `trace.rs` + `examples/script_trace.rs` put that engine in the application's default vehicle and transmission and emit `traceFrames`' 33 fields per frame, so the `RUN` gate is now two-sided: `script_reference --run` against the recorded fixture, and the port's own engine against the reference -- **all 16 263 lines identical**, physics, gas dynamics, synthesizer input and rendered audio. `graph_dump --engine` covers the parameters on their own (98 lines), `tests/run_tests.rs` the same trace without a C++ build, and `tests/engine_tests.rs` the wiring of every engine input. The drivetrain was the last M4 gap and is now closed (§8.5): `build::simulator` builds the script's own `vehicle` and `transmission` through `objects.rs`'s `TransmissionData`, `add_gear`, `set_vehicle`/`set_transmission`, falling back **per object** to the application's pair exactly as `EngineSimApplication::loadScript` does. **M5 is done**: see the row below. **Widened past `main.mr`:** `tools/parity/engines/` holds one wrapper per engine asset that builds one -- 20 of them, from a Honda TRX520 single and a five-cylinder Audi to a Merlin V12 and a nine-cylinder radial -- each imported and `main()`ed so the same `--run` trace can be taken, and all 20 are byte-identical to the original (`PASS: 20 engines`, 15 339–21 672 lines each). What they found that one engine could not: an **exhaust index** that `build.rs` never stamped, so every exhaust wrote into staging slot 0 (2 461 differing lines on the 2JZ); `__engine_sim__add_slave_journal` registered as a class but unported, so a radial's slave journals never told their rod's master, and every rod rode a journal with no crankshaft; and the `std::set` of pointers behind the intake/exhaust order, which is allocation order -- node-id order -- and only shows itself on an engine with **two** exhaust systems of different lengths (the LFA V10, the Merlin, the 412 T2) |
| M5 | ✅ | **the audio the script asks for**. `objects.rs`'s `ImpulseResponseData` is `ImpulseResponseNode::generate`: the filename resolved against the directory of the *unit that wrote the node* (`paths.rs` is `parent_path() / filename`, one of the two path rules a script can reach) and the volume the node asked for. `wav.rs` is `ysWindowsAudioWaveFile` without `mmio` -- walk the chunks, take `blockAlign / channels * 8` for the bit depth and `dataSize / blockAlign` for the frame count, and hand the synthesizer the first `frames` little-endian `int16`s, which is exactly what `EngineSimApplication::loadScript` passes it. `build` now returns `BuiltEngine` (the engine plus one response per exhaust) and `simulator` installs each file at its own volume, falling back to the deterministic table only for an exhaust that names none -- so the `RUN` gate's audio is a convolution over the real samples: **16 265 lines identical**, and the 1 376 `audio*` values the synthetic table used to produce all move. Three gates: the header's two new lines per exhaust (`exhaust<i>_impulse_response` and `..._volume`, 98 lines now), the `OBJECTS` gate's `impulse_response_filename`/`_volume` records for all ten library nodes, and `WAVE` -- `script_reference --wave` against `examples/wave_dump` on all **10 wave files**, channels, rate, bits, frames and an FNV-1a hash over every sample identical. The filename a dump prints is the repository-relative form, because the object holds the absolute one (the units are absolute) and a fixture that pinned a checkout directory would be worth nothing |
| M6 | ✅ | **the application without the window**. `crates/engine-sim-cli` parses a command line, resolves and executes the script through the port's own front end (`resolve::program` is that whole pipeline in one call: load, lower, rules, imports, definitions, circularity, expansion, `Program::new`), builds the simulator, and then runs `EngineSimApplication`'s frame loop -- `start_frame`, `simulate_step` until it returns false, `end_frame`, `render_audio_block`, `read_audio_output` -- until it has produced the requested seconds *of audio*, because a frame is not exactly `1/fps` of engine time (the latency controller scales it by 1.1/0.9, so a fixed frame count runs 2.20 s for every 2 s asked). Output is a 44.1 kHz 16-bit mono wave file and one CSV row per frame (`frame,time_s,rpm,vehicle_kmh,crank_rad_s,throttle,dyno_torque_nm,dyno_power_w,exhaust_pa,chamber0_pa,audio_samples`), plus a summary. `--gear`/`--shift`/`--clutch` are the driver's controls (§8.5). Three behaviours are the application's, not the engine's, and the CLI has to reproduce them: the audio is always rendered, because the synthesizer shares its random sequence with combustion; the starter is released after `--starter` seconds, because its velocity constraint otherwise holds the crank at the starter's speed forever; and the ignition is switched on, because `IgnitionModule` is built disabled and nothing but the `A` key ever enables it. `--no-starter`/`--no-ignition` reach the old behaviour. The gate is `DRIVEN` (see below) plus `tests/cli_tests.rs`: 7 integration tests over the binary's contract (usage, `--help`, unknown option, throttles and gears out of range, the wave file's header and length, the CSV's shape) and one that pins the ignition default by requiring the engine to idle after the starter lets go, and to stop without it |

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

### 8.4 The command line, and the frames that never fired

M6 is the first milestone whose gate is not a trace diff of a script, because
there was no headless reference to diff against: `EngineSimApplication` is a
Win32/D3D window, and the only way to run a script without it is to write the
loop. So the CLI is verified two ways instead.

**The driving is verified against the oracle.** The `.mr` harnesses had always
compared 400 frames with the starter held and the ignition off — that is what
the application does before `S` and `A` are pressed, and it is bit-exact. It is
also, as it turns out, 400 frames in which no cylinder ever fires: the whole
combustion path of the script-built engine was outside every gate. `DRIVEN`
closes that. Both harnesses take the same four environment variables
(`PARITY_THROTTLE`, `PARITY_STARTER_SECONDS`, `PARITY_IGNITION`,
`PARITY_DYNO_RPM`), and the gate drives `assets/main.mr` identically on both
sides for **20 000 frames** — two seconds of a running engine, 1 740 frames with
a lit cylinder, chamber peaks near 17 bar — and compares all **808 301**
checkpoints. It costs three seconds.

The first attempt at that comparison failed at step 3 558 by one ulp, and the
cause was in the harness, not the port: the C++ side had no `PARITY_IGNITION`,
so it ran with the module disabled while the port ran with it enabled, and the
first spark — which at that mixture returns before doing anything except consume
a `rand()` — moved the trace. Adding the variable to both sides made the two
runs identical again. It is worth writing down because the diff was *tiny*: a
1-ulp divergence at the first spark looks exactly like a floating-point
mis-transliteration, and it was not one.

**The binary is verified on its own terms.** `tests/cli_tests.rs` runs the
executable and checks what a command line promises: `--help` exits 0 and lists
the options, no script and unknown options and an out-of-range throttle or gear
are refused with the right message, a 0.2 s run writes a wave file whose RIFF
header says mono/44 100 Hz/16-bit, whose data block is exactly twice the sample
count the summary reports, and whose CSV has one row per frame. The last test is
the one that matters: it requires the engine to idle above 300 rpm a second after
the starter is released, and to stay under the starter's own speed with
`--no-ignition`. That is the ignition default pinned as behaviour rather than as
a comment.

One thing the CLI deliberately does not do: it does not chase the
application's dyno *pull* (the ramp `m_dynoSpeed += rpm(500) * dt` while the
filtered torque is positive): the sign of that condition in the original is
inverted relative to what the API reports here, so porting it needs its own
reading of the app's intent rather than a transliteration. The hold is enough to
read a torque curve one point at a time, and it is what the harness already
verifies.

### 8.5 The drivetrain, and the throttle that was never open

M4 built the engine and stopped there: `simulator()` put it in the
application's default vehicle and six-speed, whatever the script said.
`EngineSimApplication::loadScript` does not work that way — it reads
`es_script::Compiler::output()->vehicle` and `->transmission` and falls back to
its own pair **per object**, so a script may bring a car and no gearbox. Every
engine that builds one under `assets/engines/` brings both — 20 of the 27 files,
the other seven being parts libraries a wrapper composes — and `assets/main.mr` brings the
Impreza its engine was recorded from: 2700 lb, 1224.7 kg, six ratios from 3.636
to 0.756. The gates did not notice, because they never asked the car to move.

The fix is the rule itself, in `build::simulator`, plus the four builtins the
objects need: `transmission` (keeps the ratios, refreshes the clutch limit),
`add_gear`, `set_vehicle` and `set_transmission`, all of which resolve their
target through `object_source` the way the rest of the object actions do. The
`EngineContext` side is `TransmissionData` (limit plus ratios), `transmissions`
keyed by node, and `selected_vehicle`/`selected_transmission` — the two pointers
`Compiler::Output` exposes, which is exactly what "published" means.

Gating it needed three things:

* **The oracle had to learn the rule**, or the comparison would have been
  between two different cars. `script_reference`'s `loadEngineAndTrace` now
  consults the compiler's output with the same per-object fallback.
* **The object dump had to show which branch was taken.** Both dumpers append
  `O,0,selected_vehicle,<0|1>` and `O,0,selected_transmission,<0|1>` after the
  object loop. The *numbers* are not in it: `Transmission` and
  `TransmissionNode` both keep their ratios protected, and the harness is not a
  friend of either, so the ratios are gated by a driven run instead — a wrong
  ratio moves the engine.
* **The trace needed a driver.** `Drive` grew `clutch` and `gear`, applied the
  way the application applies them (`Transmission` starts at zero clutch
  pressure and in neutral, so nothing moves until a driver asks), and the
  `DRIVE` gate drives three fixtures under `tools/parity/engines/`: the Honda
  wrapper (both from the script), `vehicle_without_transmission.mr` (a 900 kg
  body in front of the application's gearbox) and
  `drivetrain_from_application.mr` (an asset's engine node called without its
  `main`, so neither object exists). All three are bit-identical over 20 000
  frames; the second is what would catch a port that paired the two pointers
  instead of falling back one at a time.

Writing the two new fixtures also exposed a hole: the IR gate parsed `es/` and
`assets/` but not `tools/parity/engines/`, and the first versions of both
fixtures were scripts the original's own resolver **rejects** — a `set_vehicle`
of a node that never returned the object, and `units.*` inside a node with no
`units units()` in scope — while the port's front end accepted both. They are
now in the IR gate, validated by `script_reference --check` like every other
script, and both bugs are gone.

The last thing the drivetrain forced into the open was the throttle. The
application's keys drive a *speed control*, and `DirectThrottleLinkage` turns it
into a plate position with `position = 1 - control^gamma`, which `Intake` reads
through `cos(position * pi/2)`: **control 1 is wide open, position 0 is wide
open**, and the keys Q/W/E/R are an opening throttle in that order. Both `.mr`
drivers — the CLI and the parity harness — had it inverted, so every run that
called itself wide open was nearly shut: the EJ25 "revved" to 790 rpm with 16 bar
chamber peaks, and the dyno reported 240 Nm of *reaction* to drag it round. A
comparison cannot catch a mistake made identically on both sides, which is why it
survived until an engine was asked to move a car. Corrected on both sides, wide
open from the starter takes the EJ25 to 6855 rpm, and `DRIVEN` now compares a
running engine at wide-open throttle rather than a coasting one.
