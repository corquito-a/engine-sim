#!/usr/bin/env python3
"""Generate `src/language_rules.rs` from the two `registerBuiltinNodeTypes` bodies.

The expansion phase asks the language rules four questions -- which builtin node
stands behind a literal, behind a binary operator applied to two channel types,
behind a unary operator, and behind a conversion -- and the answers are a table
of `registerLiteralType` / `registerOperator` / `registerUnaryOperator` /
`registerConversion` calls in:

    scripting/src/language_rules.cpp                 (the engine's rules)
    dependencies/submodules/piranha/test/test_rules.cpp   (the fixtures' rules)

Transcribing those by hand is how a table silently loses a row, so this script
parses them and the crate's test suite re-parses them and compares (see
`language_rules::tests`). Every call site has to produce exactly one row; the
counts are asserted here as well so a parse that stops early cannot write a
short table.

    python3 rust/tools/parity/extract_language_rules.py [--check]

The fifth table -- the ports of each builtin's *reference node* -- cannot be read
out of the sources: a reference node is a real `Node` instance whose ports are
registered from constructors, so the answers come from running the original.
They are recorded into `tools/parity/builtin_ports.txt` by
`record_builtin_ports.sh` and folded in here.

`--check` compares the file on disk with what would be generated and exits
non-zero if they differ.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUTPUT = ROOT / "rust/crates/engine-sim-script/src/language_rules.rs"

SOURCES = {
    "ENGINE_SIM": ROOT / "scripting/src/language_rules.cpp",
    "PIRANHA": ROOT / "dependencies/submodules/piranha/test/test_rules.cpp",
}

PORTS = ROOT / "rust/tools/parity/builtin_ports.txt"

FUNDAMENTAL = ROOT / "dependencies/submodules/piranha/src/fundamental_types.cpp"
OBJECT_CHANNELS = ROOT / "scripting/src/channel_types.cpp"

# `Operator::Dot` is not a rule: `.` never expands through the operator table.
BIN_OPS = ["Dot", "Add", "Sub", "Mul", "Div"]
UN_OPS = ["Default", "NumericNegate", "BoolNegate", "Positive"]
LITERALS = ["Float", "String", "Integer", "Boolean"]


def strip_comments(source: str) -> str:
    source = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return re.sub(r"//[^\n]*", "", source)


def channel_type_names() -> dict[str, str]:
    """`FundamentalType::FloatType` -> `FloatNodeType`, and the object channels.

    `piranha::FundamentalType::FloatType` is constructed with the type string
    `"FloatNodeType"`; `es_script::ObjectChannel::FooChannel` is constructed by
    `DEFINE_CHANNEL(FooChannel)`, which stringifies the identifier.
    """
    names = {}

    fundamental = FUNDAMENTAL.read_text()
    for match in re.finditer(
        r'const piranha::ChannelType piranha::FundamentalType::(\w+)\("([^"]+)"\);',
        fundamental,
    ):
        names[match.group(1)] = match.group(2)

    channels = OBJECT_CHANNELS.read_text()
    for match in re.finditer(r"DEFINE_CHANNEL\((\w+)\);", channels):
        names[match.group(1)] = match.group(1)

    assert names, "no channel types found"
    return names


def normalise_type(field: str, names: dict[str, str]) -> str:
    """`&piranha::FundamentalType::FloatType` -> `FloatNodeType`."""
    field = field.strip().lstrip("&").strip()
    assert "::" in field, f"cannot make sense of a type field: {field!r}"
    field = field.rsplit("::", 1)[1]
    assert field in names, f"{field} is not a channel type this port knows"
    return names[field]


def parse(source: str, names: dict[str, str]) -> dict[str, list[tuple]]:
    source = strip_comments(source)

    # `registerBuiltinType<...>("name", &Type)` -- only the ones that name a
    # channel type; the rest have none (`BuiltinTypeInfo::nodeType` is null).
    builtins = []
    for match in re.finditer(
        r'registerBuiltinType\s*<[^;]*?>\s*\(\s*"([^"]+)"\s*,\s*&([^)]+)\)', source
    ):
        builtins.append((match.group(1), normalise_type(match.group(2), names)))
    assert len(builtins) == len(
        re.findall(r"registerBuiltinType\s*<[^;]*?>\s*\([^;]*?,\s*&", source)
    ), "a registerBuiltinType call with a channel type was not parsed"

    def count(name: str) -> int:
        return len(re.findall(rf"\b{name}\s*\(", source))

    literals = []
    for match in re.finditer(
        r'registerLiteralType\(\s*\w+::LiteralType::(\w+)\s*,\s*"([^"]+)"\s*\)', source
    ):
        literal, builtin = match.groups()
        assert literal in LITERALS, f"unknown literal type {literal}"
        literals.append((literal, builtin))
    assert len(literals) == count("registerLiteralType"), "a registerLiteralType call was not parsed"

    operators = []
    for match in re.finditer(
        r'registerOperator\(\s*\{\s*([^}]*)\}\s*,\s*"([^"]+)"\s*\)', source
    ):
        fields = [field.strip() for field in match.group(1).split(",") if field.strip()]
        assert len(fields) == 3, f"unexpected operator mapping: {match.group(0)!r}"
        op = fields[0].rsplit("::", 1)[1]
        assert op in BIN_OPS, f"unknown binary operator {op}"
        operators.append((op, normalise_type(fields[1], names), normalise_type(fields[2], names), match.group(2)))
    assert len(operators) == count("registerOperator"), "a registerOperator call was not parsed"

    unary = []
    for match in re.finditer(
        r'registerUnaryOperator\(\s*\{\s*([^}]*)\}\s*,\s*"([^"]+)"\s*\)', source
    ):
        fields = [field.strip() for field in match.group(1).split(",") if field.strip()]
        assert len(fields) == 2, f"unexpected unary mapping: {match.group(0)!r}"
        op = fields[0].rsplit("::", 1)[1]
        assert op in UN_OPS, f"unknown unary operator {op}"
        unary.append((op, normalise_type(fields[1], names), match.group(2)))
    assert len(unary) == count("registerUnaryOperator"), "a registerUnaryOperator call was not parsed"

    conversions = []
    for match in re.finditer(
        r'registerConversion\(\s*\{\s*([^}]*)\}\s*,\s*"([^"]+)"\s*\)', source
    ):
        fields = [field.strip() for field in match.group(1).split(",") if field.strip()]
        assert len(fields) == 2, f"unexpected conversion: {match.group(0)!r}"
        conversions.append((normalise_type(fields[0], names), normalise_type(fields[1], names), match.group(2)))
    assert len(conversions) == count("registerConversion"), "a registerConversion call was not parsed"

    return {
        "builtins": builtins,
        "literals": literals,
        "operators": operators,
        "unary": unary,
        "conversions": conversions,
    }


def parse_ports() -> dict[str, dict[str, list[tuple]]]:
    """The recorded reference-node ports, per rule set.

    `input,<builtin>,<port>,<modifiesInput>,<enableInput>` and
    `output,<builtin>,<port>,<isAlias>`, with `absent,<builtin>` for a name the
    rules do not register (which no rule set currently does).
    """
    tables: dict[str, dict[str, list[tuple]]] = {}
    current = None

    for line in PORTS.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue

        if line.startswith("[") and line.endswith("]"):
            current = line[1:-1].upper()
            tables[current] = {"builtins": [], "inputs": [], "outputs": [], "absent": []}
            continue

        assert current is not None, f"a row before any rule set: {line!r}"
        fields = line.split(",")

        if fields[0] == "builtin":
            assert len(fields) == 2, line
            tables[current]["builtins"].append(fields[1])
        elif fields[0] == "absent":
            assert len(fields) == 2, line
            tables[current]["absent"].append(fields[1])
        elif fields[0] == "input":
            assert len(fields) == 5, line
            tables[current]["inputs"].append(
                (fields[1], fields[2], fields[3] == "1", fields[4] == "1")
            )
        elif fields[0] == "output":
            assert len(fields) == 4, line
            tables[current]["outputs"].append((fields[1], fields[2], fields[3] == "1"))
        else:
            raise AssertionError(f"unknown port row {line!r}")

    assert set(tables) == {"ENGINE_SIM", "PIRANHA"}, sorted(tables)

    for name, table in tables.items():
        assert not table["absent"], f"{name} has unregistered builtin names"
        assert table["builtins"], name

    return tables


def render(tables: dict[str, dict[str, list[tuple]]], ports: dict[str, dict[str, list[tuple]]]) -> str:
    lines = [
        "//! The language rules' literal, operator and conversion table.",
        "//!",
        "//! Generated by `rust/tools/parity/extract_language_rules.py` from",
        "//! `scripting/src/language_rules.cpp` and `piranha/test/test_rules.cpp`;",
        "//! do not edit. `language_rules::tests::the_tables_match_the_sources`",
        "//! re-parses both files and fails if this table has drifted.",
        "",
        "use crate::ir::{BinOp, UnOp};",
        "",
        "/// `piranha::LiteralType`",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub enum LiteralKind {",
        "    Float,",
        "    String,",
        "    Integer,",
        "    Boolean,",
        "}",
        "",
        "/// One `registerLiteralType` call.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct LiteralRule {",
        "    pub literal: LiteralKind,",
        "    pub builtin: &'static str,",
        "}",
        "",
        "/// One `registerOperator` call: `op` applied to `left` and `right`.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct OperatorRule {",
        "    pub op: BinOp,",
        "    pub left: &'static str,",
        "    pub right: &'static str,",
        "    pub builtin: &'static str,",
        "}",
        "",
        "/// One `registerUnaryOperator` call.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct UnaryRule {",
        "    pub op: UnOp,",
        "    pub type_name: &'static str,",
        "    pub builtin: &'static str,",
        "}",
        "",
        "/// One `registerConversion` call.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct ConversionRule {",
        "    pub input: &'static str,",
        "    pub output: &'static str,",
        "    pub builtin: &'static str,",
        "}",
        "",
    ]

    for declaration in [
        "/// One port of a builtin's reference node, as `Node::getInputPortInfo`",
        "/// reports it (`IrNodeDefinition::validateBuiltinMappings`, `V0080`).",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct BuiltinInputPort {",
        "    pub name: &'static str,",
        "    pub modifies_input: bool,",
        "    pub toggle: bool,",
        "}",
        "",
        "/// One output of a builtin's reference node, as `Node::getOutputPortInfo`",
        "/// reports it (`V0081`, and the alias flag behind `V0110`).",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct BuiltinOutputPort {",
        "    pub name: &'static str,",
        "    pub alias: bool,",
        "}",
        "",
        "/// The ports a builtin type's reference node was built with.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub struct BuiltinPorts {",
        "    pub builtin: &'static str,",
        "    pub inputs: &'static [BuiltinInputPort],",
        "    pub outputs: &'static [BuiltinOutputPort],",
        "}",
        "",
    ]:
        lines.append(declaration)

    lines.append("/// `registerBuiltinType<...>(\"name\", &ChannelType)` -- the builtins")
    lines.append("/// that name a channel type, in registration order.")
    lines.append("#[derive(Clone, Copy, Debug, PartialEq, Eq)]")
    lines.append("pub struct BuiltinChannelType {")
    lines.append("    pub builtin: &'static str,")
    lines.append("    pub channel_type: &'static str,")
    lines.append("}")
    lines.append("")

    for set_name in ("ENGINE_SIM", "PIRANHA"):
        table = tables[set_name]

        lines.append(f"pub const {set_name}_BUILTIN_CHANNEL_TYPES: &[BuiltinChannelType] = &[")
        for builtin, channel_type in table["builtins"]:
            lines.append(
                f'    BuiltinChannelType {{ builtin: "{builtin}", channel_type: "{channel_type}" }},'
            )
        lines.append("];")
        lines.append("")

        lines.append(f"pub const {set_name}_LITERALS: &[LiteralRule] = &[")
        for literal, builtin in table["literals"]:
            lines.append(f'    LiteralRule {{ literal: LiteralKind::{literal}, builtin: "{builtin}" }},')
        lines.append("];")
        lines.append("")

        lines.append(f"pub const {set_name}_OPERATORS: &[OperatorRule] = &[")
        for op, left, right, builtin in table["operators"]:
            lines.append(
                f'    OperatorRule {{ op: BinOp::{op}, left: "{left}", right: "{right}", builtin: "{builtin}" }},'
            )
        lines.append("];")
        lines.append("")

        lines.append(f"pub const {set_name}_UNARY: &[UnaryRule] = &[")
        for op, type_name, builtin in table["unary"]:
            lines.append(
                f'    UnaryRule {{ op: UnOp::{op}, type_name: "{type_name}", builtin: "{builtin}" }},'
            )
        lines.append("];")
        lines.append("")

        lines.append(f"pub const {set_name}_CONVERSIONS: &[ConversionRule] = &[")
        for source, target, builtin in table["conversions"]:
            lines.append(
                f'    ConversionRule {{ input: "{source}", output: "{target}", builtin: "{builtin}" }},'
            )
        lines.append("];")
        lines.append("")

        # The port table, keyed by the builtins the dump listed -- a reference
        # node with no ports is a row with two empty slices, not a missing row.
        grouped: dict[str, dict[str, list[tuple]]] = {}
        for builtin in ports[set_name]["builtins"]:
            grouped.setdefault(builtin, {"inputs": [], "outputs": []})
        for builtin, port, modifies, toggle in ports[set_name]["inputs"]:
            assert builtin in grouped, builtin
            grouped[builtin]["inputs"].append((port, modifies, toggle))
        for builtin, port, alias in ports[set_name]["outputs"]:
            assert builtin in grouped, builtin
            grouped[builtin]["outputs"].append((port, alias))

        lines.append(f"pub const {set_name}_BUILTIN_PORTS: &[BuiltinPorts] = &[")
        for builtin, entry in grouped.items():
            lines.append("    BuiltinPorts {")
            lines.append(f'        builtin: "{builtin}",')
            lines.append("        inputs: &[")
            for port, modifies, toggle in entry["inputs"]:
                lines.append(
                    f'            BuiltinInputPort {{ name: "{port}", '
                    f"modifies_input: {str(modifies).lower()}, toggle: {str(toggle).lower()} }},"
                )
            lines.append("        ],")
            lines.append("        outputs: &[")
            for port, alias in entry["outputs"]:
                lines.append(
                    f'            BuiltinOutputPort {{ name: "{port}", alias: {str(alias).lower()} }},'
                )
            lines.append("        ],")
            lines.append("    },")
        lines.append("];")
        lines.append("")

    return "\n".join(lines) + "\n"


def main() -> int:
    names = channel_type_names()
    tables = {name: parse(path.read_text(), names) for name, path in SOURCES.items()}
    ports = parse_ports()
    rendered = render(tables, ports)

    if "--check" in sys.argv[1:]:
        if OUTPUT.read_text() != rendered:
            print(f"{OUTPUT} is out of date; re-run this script", file=sys.stderr)
            return 1
        return 0

    OUTPUT.write_text(rendered)

    for name, table in tables.items():
        print(
            f"{name}: {len(table['builtins'])} builtin channel types, "
            f"{len(table['literals'])} literals, {len(table['operators'])} operators, "
            f"{len(table['unary'])} unary, {len(table['conversions'])} conversions, "
            f"{len(ports[name]['builtins'])} builtin port tables"
        )

    return 0


if __name__ == "__main__":
    sys.exit(main())
