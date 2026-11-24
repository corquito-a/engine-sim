//! The generated language-rules table is checked against the C++ it came from.
//!
//! `rust/tools/parity/extract_language_rules.py` writes
//! `src/language_rules.rs`; this test re-derives the same four tables from
//! `scripting/src/language_rules.cpp` and `piranha/test/test_rules.cpp` and
//! compares them row for row. A `registerOperator` call that the generator
//! missed would therefore fail the build rather than silently weaken operator
//! expansion -- the same guard `builtin_types` uses for the builtin-node table.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine_sim_script::ir::{BinOp, UnOp};
use engine_sim_script::language_rules::{
    LiteralKind, ENGINE_SIM_CONVERSIONS, ENGINE_SIM_LITERALS, ENGINE_SIM_OPERATORS, ENGINE_SIM_UNARY,
    PIRANHA_CONVERSIONS, PIRANHA_LITERALS, PIRANHA_OPERATORS, PIRANHA_UNARY,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// Comments out of the way: a `registerOperator` inside a comment is not a rule.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;

    while let Some(index) = rest.find("//").or_else(|| rest.find("/*")) {
        match &rest[index..index + 2] {
            "//" => {
                let end = rest[index..].find('\n').map(|offset| index + offset).unwrap_or(rest.len());
                out.push_str(&rest[..index]);
                rest = &rest[end..];
            }
            _ => {
                let end = rest[index..]
                    .find("*/")
                    .map(|offset| index + offset + 2)
                    .unwrap_or(rest.len());
                out.push_str(&rest[..index]);
                rest = &rest[end..];
            }
        }
    }

    out.push_str(rest);
    out
}

/// The argument text of every `name(...)` call, with nested brackets balanced.
fn calls<'a>(source: &'a str, name: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut search = source;

    while let Some(index) = search.find(name) {
        let after = &search[index + name.len()..];
        let trimmed = after.trim_start();
        if !trimmed.starts_with('(') {
            search = &search[index + name.len()..];
            continue;
        }

        let open = after.len() - trimmed.len();
        let mut depth = 0usize;
        let mut close = None;
        for (offset, character) in after[open..].char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + offset);
                        break;
                    }
                }
                _ => {}
            }
        }

        let close = close.expect("unbalanced call");
        found.push(&after[open + 1..close]);
        search = &search[index + name.len()..];
    }

    found
}

/// The quoted string at the end of an argument list.
fn builtin_name(arguments: &str) -> String {
    let start = arguments.rfind('"').and_then(|end| {
        arguments[..end]
            .rfind('"')
            .map(|start| (start, end))
    });

    let (start, end) = start.expect("a builtin name");
    arguments[start + 1..end].to_string()
}

/// `&piranha::FundamentalType::FloatType` -> `FloatNodeType`.
fn channel_type(field: &str, names: &BTreeMap<String, String>) -> String {
    let field = field.trim().trim_start_matches('&').trim();
    let tail = field.rsplit("::").next().expect("a type");
    names
        .get(tail)
        .unwrap_or_else(|| panic!("{tail} is not a channel type the port knows"))
        .clone()
}

fn channel_types() -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();

    for line in read("dependencies/submodules/piranha/src/fundamental_types.cpp").lines() {
        let line = line.trim();
        if let (Some(rest), true) = (line.strip_prefix("const piranha::ChannelType piranha::FundamentalType::"), line.ends_with(");")) {
            let (name, rest) = rest.split_once("(\"").expect("a constructor");
            let type_string = rest.trim_end_matches(");").trim_end_matches('"');
            names.insert(name.to_string(), type_string.to_string());
        }
    }

    for line in read("scripting/src/channel_types.cpp").lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("DEFINE_CHANNEL(") {
            let name = rest.trim_end_matches(");");
            names.insert(name.to_string(), name.to_string());
        }
    }

    names
}

fn literal_kind(name: &str) -> LiteralKind {
    match name {
        "Float" => LiteralKind::Float,
        "String" => LiteralKind::String,
        "Integer" => LiteralKind::Integer,
        "Boolean" => LiteralKind::Boolean,
        other => panic!("unknown literal type {other}"),
    }
}

fn binary_operator(name: &str) -> BinOp {
    match name {
        "Dot" => BinOp::Dot,
        "Add" => BinOp::Add,
        "Sub" => BinOp::Sub,
        "Mul" => BinOp::Mul,
        "Div" => BinOp::Div,
        other => panic!("unknown binary operator {other}"),
    }
}

fn unary_operator(name: &str) -> UnOp {
    match name {
        "Default" => UnOp::Default,
        "NumericNegate" => UnOp::NumericNegate,
        "BoolNegate" => UnOp::BoolNegate,
        "Positive" => UnOp::Positive,
        other => panic!("unknown unary operator {other}"),
    }
}

/// The braced field list at the start of an argument list.
fn fields(arguments: &str) -> Vec<String> {
    let open = arguments.find('{').expect("a mapping");
    let close = arguments[open..].find('}').expect("a closing brace") + open;

    arguments[open + 1..close]
        .split(',')
        .map(|field| field.trim().to_string())
        .filter(|field| !field.is_empty())
        .collect()
}

#[derive(Debug, PartialEq)]
struct Tables {
    literals: Vec<(LiteralKind, String)>,
    operators: Vec<(BinOp, String, String, String)>,
    unary: Vec<(UnOp, String, String)>,
    conversions: Vec<(String, String, String)>,
}

fn parse(source: &str, names: &BTreeMap<String, String>) -> Tables {
    let source = strip_comments(source);

    let literals = calls(&source, "registerLiteralType")
        .into_iter()
        .map(|arguments| {
            let comma = arguments.find(',').expect("a literal type");
            let literal = arguments[..comma].trim().rsplit("::").next().unwrap();
            (literal_kind(literal), builtin_name(arguments))
        })
        .collect();

    let operators = calls(&source, "registerOperator")
        .into_iter()
        .map(|arguments| {
            let fields = fields(arguments);
            assert_eq!(fields.len(), 3, "an operator mapping has three members");
            (
                binary_operator(fields[0].rsplit("::").next().unwrap()),
                channel_type(&fields[1], names),
                channel_type(&fields[2], names),
                builtin_name(arguments),
            )
        })
        .collect();

    let unary = calls(&source, "registerUnaryOperator")
        .into_iter()
        .map(|arguments| {
            let fields = fields(arguments);
            assert_eq!(fields.len(), 2, "a unary mapping has two members");
            (
                unary_operator(fields[0].rsplit("::").next().unwrap()),
                channel_type(&fields[1], names),
                builtin_name(arguments),
            )
        })
        .collect();

    let conversions = calls(&source, "registerConversion")
        .into_iter()
        .map(|arguments| {
            let fields = fields(arguments);
            assert_eq!(fields.len(), 2, "a conversion has two members");
            (
                channel_type(&fields[0], names),
                channel_type(&fields[1], names),
                builtin_name(arguments),
            )
        })
        .collect();

    Tables { literals, operators, unary, conversions }
}

fn generated_literals(rules: engine_sim_script::Rules) -> Vec<(LiteralKind, String)> {
    let table = match rules {
        engine_sim_script::Rules::EngineSim => ENGINE_SIM_LITERALS,
        engine_sim_script::Rules::Piranha => PIRANHA_LITERALS,
    };

    table.iter().map(|rule| (rule.literal, rule.builtin.to_string())).collect()
}

fn generated_operators(rules: engine_sim_script::Rules) -> Vec<(BinOp, String, String, String)> {
    let table = match rules {
        engine_sim_script::Rules::EngineSim => ENGINE_SIM_OPERATORS,
        engine_sim_script::Rules::Piranha => PIRANHA_OPERATORS,
    };

    table
        .iter()
        .map(|rule| {
            (rule.op, rule.left.to_string(), rule.right.to_string(), rule.builtin.to_string())
        })
        .collect()
}

fn generated_unary(rules: engine_sim_script::Rules) -> Vec<(UnOp, String, String)> {
    let table = match rules {
        engine_sim_script::Rules::EngineSim => ENGINE_SIM_UNARY,
        engine_sim_script::Rules::Piranha => PIRANHA_UNARY,
    };

    table
        .iter()
        .map(|rule| (rule.op, rule.type_name.to_string(), rule.builtin.to_string()))
        .collect()
}

fn generated_conversions(rules: engine_sim_script::Rules) -> Vec<(String, String, String)> {
    let table = match rules {
        engine_sim_script::Rules::EngineSim => ENGINE_SIM_CONVERSIONS,
        engine_sim_script::Rules::Piranha => PIRANHA_CONVERSIONS,
    };

    table
        .iter()
        .map(|rule| (rule.input.to_string(), rule.output.to_string(), rule.builtin.to_string()))
        .collect()
}

fn check(rules: engine_sim_script::Rules, relative: &str) {
    let names = channel_types();
    let parsed = parse(&read(relative), &names);

    // `calls` counts every call site, so a table that lost a row fails here even
    // before the element comparison.
    assert_eq!(parsed.literals, generated_literals(rules), "{relative}: literal rules");
    assert_eq!(parsed.operators, generated_operators(rules), "{relative}: operator rules");
    assert_eq!(parsed.unary, generated_unary(rules), "{relative}: unary rules");
    assert_eq!(parsed.conversions, generated_conversions(rules), "{relative}: conversions");
}

#[test]
fn the_engine_sim_tables_match_the_sources() {
    check(engine_sim_script::Rules::EngineSim, "scripting/src/language_rules.cpp");
}

#[test]
fn the_piranha_tables_match_the_sources() {
    check(engine_sim_script::Rules::Piranha, "dependencies/submodules/piranha/test/test_rules.cpp");
}

#[test]
fn the_tables_are_not_empty() {
    assert_eq!(ENGINE_SIM_LITERALS.len(), 4);
    assert_eq!(PIRANHA_LITERALS.len(), 4);
    assert!(!ENGINE_SIM_OPERATORS.is_empty());
    assert!(!PIRANHA_OPERATORS.is_empty());
}
