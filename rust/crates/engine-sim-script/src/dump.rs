//! S-expression serialisation of the parsed IR.
//!
//! The format exists so the C++ reference oracle can rebuild the *same*
//! piranha IR objects the original parser would have built: the oracle cannot
//! run flex/bison in this environment, but it can run everything downstream of
//! the parser. Emitting a small, regular text format keeps that bridge honest
//! and readable, and lets the Rust and C++ sides be diffed as text.
//!
//! Token flags: only `specified` is emitted. `IrTokenInfo::empty` is never read
//! by piranha or the scripting layer (`grep -r "\.empty\b"` finds nothing), and
//! `valid` is only used to pick the error code in `Parser::error`.

use crate::ir::*;

/// Escapes an atom so it survives a round trip through the reader.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_token(text: &mut String, lines: bool, token: &Token) {
    if lines {
        // The conformance harness needs `(code, line)`; the original's own test
        // suite asserts line numbers (`findError(list, code, 18)`). Off by
        // default so the IR round-trip grammar and its tests are unchanged.
        // A specified token with no source position (`Token::new`, e.g. the
        // `this` attribute `IrNode::setThis` synthesises) is `IrTokenInfo()`
        // in the original: `lineStart == INT_MAX`, a span that cannot lower a
        // summary's line. Writing 0 here would make the oracle fold a line 0
        // into every summary it touches.
        let line = token.line_start();

        text.push_str(&format!(
            "(tok {} {} {})",
            quote(&token.data),
            if token.specified { 1 } else { 0 },
            line
        ));
    }
    else {
        text.push_str(&format!(
            "(tok {} {})",
            quote(&token.data),
            if token.specified { 1 } else { 0 }
        ));
    }
}

fn float(value: f64) -> String {
    // `{:?}` is Rust's shortest round-tripping representation; `std::stod` on
    // the oracle side parses it back to the identical bit pattern.
    if value.is_finite() {
        format!("{:?}", value)
    }
    else {
        // Not reachable from `.mr` (the scanner has no exponent or `inf`
        // syntax) but keeps the writer total.
        quote(&value.to_string())
    }
}

/// `(int <value>)`, or `(int (tok "42" 1 15) 42)` when lines are carried: the
/// token is what makes a summary span the literal, as it does in the original.
fn write_constant(text: &mut String, lines: bool, name: &str, token: &Token, value: &str) {
    text.push('(');
    text.push_str(name);
    text.push(' ');

    if lines {
        write_token(text, lines, token);
        text.push(' ');
    }

    text.push_str(value);
    text.push(')');
}

fn write_value(text: &mut String, lines: bool, value: &Value) {
    match value {
        Value::Int { token, value } => write_constant(text, lines, "int", token, &value.to_string()),
        Value::Float { token, value: v } => write_constant(text, lines, "float", token, &float(*v)),
        Value::Bool { token, value } => {
            write_constant(text, lines, "bool", token, if *value { "1" } else { "0" })
        }
        Value::Str { token, value } => write_constant(text, lines, "str", token, &quote(value)),
        Value::Label { token: t } => {
            text.push_str("(label ");
            write_token(text, lines, t);
            text.push(')');
        }
        Value::NodeRef { node } => {
            text.push_str("(nref ");
            write_node(text, lines, node);
            text.push(')');
        }
        Value::Binary { token, op, left, right } => {
            let name = match op {
                BinOp::Dot => "dot",
                BinOp::Add => "add",
                BinOp::Sub => "sub",
                BinOp::Mul => "mul",
                BinOp::Div => "div",
            };
            text.push_str(&format!("(bin {} ", name));
            if lines {
                write_token(text, lines, token);
                text.push(' ');
            }
            write_value(text, lines, left);
            text.push(' ');
            write_value(text, lines, right);
            text.push(')');
        }
        Value::Unary { token, op, operand } => {
            let name = match op {
                UnOp::Default => "default",
                UnOp::NumericNegate => "neg",
                UnOp::BoolNegate => "not",
                UnOp::Positive => "pos",
            };
            text.push_str(&format!("(un {} ", name));
            if lines {
                write_token(text, lines, token);
                text.push(' ');
            }
            write_value(text, lines, operand);
            text.push(')');
        }
    }
}

fn write_node(text: &mut String, lines: bool, node: &Node) {
    text.push_str("(node (type ");
    write_token(text, lines, &node.type_name);
    text.push_str(") (ns ");
    write_token(text, lines, &node.library);
    text.push_str(") (name ");
    write_token(text, lines, &node.name);
    text.push(')');

    text.push_str(" (attrs");
    for attribute in &node.attributes {
        text.push_str(" (attr ");
        write_token(text, lines, &attribute.name);
        text.push(' ');
        write_value(text, lines, &attribute.value);
        text.push(')');
    }
    text.push_str("))");
}

fn write_port(text: &mut String, lines: bool, port: &AttributeDefinition) {
    // The direction keyword is a token like any other: it is what `V0090`/
    // `V0100` are reported at, and it is a registered token of the definition,
    // so it moves the summary token's line.
    text.push_str("(port (dir ");
    write_token(text, lines, &port.direction_token);
    text.push_str(") (name ");
    write_token(text, lines, &port.name);
    text.push_str(&format!(") (alias {})", if port.is_alias { 1 } else { 0 }));

    if port.is_alias {
        text.push_str(" (alias_tok ");
        write_token(text, lines, &port.alias_token);
        text.push(')');
    }

    if let Some((library, type_name)) = &port.type_info {
        text.push_str(" (type (ns ");
        write_token(text, lines, library);
        text.push_str(") (nm ");
        write_token(text, lines, type_name);
        text.push_str("))");
    }

    if let Some(default) = &port.default_value {
        text.push_str(" (default ");
        write_value(text, lines, default);
        text.push(')');
    }

    text.push(')');
}

fn write_definition(text: &mut String, lines: bool, definition: &NodeDefinition, is_auto: bool) {
    text.push_str("(def (name ");
    write_token(text, lines, &definition.name);
    text.push_str(&format!(
        ") (inline {}) (vis {}) (auto {})",
        if definition.is_inline { 1 } else { 0 },
        definition.visibility.as_str(),
        if is_auto { 1 } else { 0 }
    ));

    if definition.defines_builtin {
        text.push_str(" (builtin ");
        write_token(text, lines, &definition.builtin_name);
        text.push(')');
    }

    if let Some(scope) = &definition.scope_token {
        text.push_str(" (scope ");
        write_token(text, lines, scope);
        text.push(')');
    }

    text.push_str(" (ports");
    for port in &definition.ports {
        text.push(' ');
        write_port(text, lines, port);
    }
    text.push(')');

    if let Some(body) = &definition.body {
        text.push_str(" (body");
        for node in body {
            text.push(' ');
            write_node(text, lines, node);
        }
        text.push(')');
    }

    text.push(')');
}

fn write_import(text: &mut String, lines: bool, import: &ImportStatement) {
    text.push_str(&format!("(import (vis {}) (lib ", import.visibility.as_str()));
    write_token(text, lines, &import.lib_name);
    text.push(')');
    if let Some(short_name) = &import.short_name {
        text.push_str(" (short ");
        write_token(text, lines, short_name);
        text.push(')');
    }
    text.push_str(")\n");
}

fn write_statement(text: &mut String, lines: bool, statement: &Statement) {
    match statement {
        Statement::Import(import) => write_import(text, lines, import),
        Statement::Definition(definition_) => {
            text.push_str("  ");
            write_definition(text, lines, definition_, false);
            text.push('\n');
        }
        Statement::AutoDefinition(definition_) => {
            // `auto node ...` instantiates a node and registers the definition;
            // the instance is implied by the flag, so only the definition is
            // written here.
            text.push_str("  ");
            write_definition(text, lines, definition_, true);
            text.push('\n');
        }
        Statement::Node(node) => {
            text.push_str("  ");
            write_node(text, lines, node);
            text.push('\n');
        }
    }
}

/// Serialises every unit, one `(unit "path" ...)` form per file.
pub fn dump_units(units: &[Unit]) -> String {
    dump_units_with(units, false)
}

/// As [`dump_units`], optionally carrying token line numbers.
pub fn dump_units_with(units: &[Unit], lines: bool) -> String {
    let mut text = String::new();

    for unit in units {
        text.push_str(&format!("(unit {}\n", quote(&unit.path)));

        // `IrCompilationUnit` keeps imports, definitions and nodes in three
        // separate lists, so the dump does too: that is the structure the IR
        // actually has, and it makes the round trip through the oracle exact.
        for statement in &unit.statements {
            if let Statement::Import(_) = statement {
                write_statement(&mut text, lines, statement);
            }
        }

        for statement in &unit.statements {
            if matches!(statement, Statement::Definition(_) | Statement::AutoDefinition(_)) {
                write_statement(&mut text, lines, statement);
            }
        }

        for statement in &unit.statements {
            if let Statement::Node(_) = statement {
                write_statement(&mut text, lines, statement);
            }
        }

        text.push_str(")\n");
    }

    text
}

/// Serialises a single unit.
pub fn dump_unit(unit: &Unit) -> String {
    dump_units(std::slice::from_ref(unit))
}
