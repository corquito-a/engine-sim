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
        "literals": literals,
        "operators": operators,
        "unary": unary,
        "conversions": conversions,
    }


def render(tables: dict[str, dict[str, list[tuple]]]) -> str:
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

    for set_name in ("ENGINE_SIM", "PIRANHA"):
        table = tables[set_name]
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

    return "\n".join(lines) + "\n"


def main() -> int:
    names = channel_type_names()
    tables = {name: parse(path.read_text(), names) for name, path in SOURCES.items()}
    rendered = render(tables)

    if "--check" in sys.argv[1:]:
        if OUTPUT.read_text() != rendered:
            print(f"{OUTPUT} is out of date; re-run this script", file=sys.stderr)
            return 1
        return 0

    OUTPUT.write_text(rendered)

    for name, table in tables.items():
        print(
            f"{name}: {len(table['literals'])} literals, {len(table['operators'])} operators, "
            f"{len(table['unary'])} unary, {len(table['conversions'])} conversions"
        )

    return 0


if __name__ == "__main__":
    sys.exit(main())
