//! Lexer and parser behaviour, pinned against `specification.y` and
//! `scanner.l`.

use engine_sim_script::ir::*;
use engine_sim_script::lexer::{tokenize, Kind, Literal};
use engine_sim_script::parser::parse_unit;

fn unit(source: &str) -> Unit {
    parse_unit("test.mr", source).expect("the script should parse")
}

fn error(source: &str) -> engine_sim_script::ParseError {
    parse_unit("test.mr", source).expect_err("the script should be rejected")
}

fn kinds(source: &str) -> Vec<Kind> {
    tokenize(source).iter().map(|token| token.kind).collect()
}

fn definition(unit: &Unit, index: usize) -> &NodeDefinition {
    unit.statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Definition(definition) | Statement::AutoDefinition(definition) => {
                Some(definition)
            }
            _ => None,
        })
        .nth(index)
        .expect("definition")
}

fn nodes(unit: &Unit) -> Vec<&Node> {
    unit.statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Node(node) => Some(node),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------- scanner

#[test]
fn keywords_lose_to_the_longest_label_match() {
    // flex prefers the longest match, so `importx` is a label, not `import`.
    assert_eq!(kinds("importx"), vec![Kind::Label, Kind::End]);
    assert_eq!(kinds("import"), vec![Kind::Import, Kind::End]);
}

#[test]
fn floats_win_over_ints_and_dots() {
    assert_eq!(
        kinds("1.5 1. 0x1F 42"),
        vec![
            Kind::Float,
            Kind::Int,
            Kind::Dot,
            Kind::Int,
            Kind::Int,
            Kind::End
        ]
    );

    let tokens = tokenize("1.5 0x1f 42");
    assert_eq!(tokens[0].literal, Literal::Float(1.5));
    assert_eq!(tokens[1].literal, Literal::Int(31));
    assert_eq!(tokens[2].literal, Literal::Int(42));
}

#[test]
fn ints_are_truncated_to_32_bits_like_the_original() {
    // `stoul` into an `unsigned int` and then cast to `int`.
    let tokens = tokenize("4294967295");
    assert_eq!(tokens[0].literal, Literal::Int(-1));
}

#[test]
fn comments_are_skipped() {
    assert_eq!(
        kinds("a // b\n/* c\nd */ e"),
        vec![Kind::Label, Kind::Label, Kind::End]
    );
}

#[test]
fn strings_keep_their_escapes_verbatim() {
    // The scanner strips the quotes only; `validateData` decodes later.
    let tokens = tokenize(r#""a\nb" "\"" "#);
    match &tokens[0].literal {
        Literal::Str(text) => assert_eq!(text, "a\\nb"),
        other => panic!("expected a string, got {:?}", other),
    }
    match &tokens[1].literal {
        Literal::Str(text) => assert_eq!(text, "\\\""),
        other => panic!("expected a string, got {:?}", other),
    }
}

#[test]
fn two_character_operators_are_not_split() {
    assert_eq!(
        kinds("=> :: = :"),
        vec![
            Kind::BuiltinPointer,
            Kind::NamespacePointer,
            Kind::Eq,
            Kind::Colon,
            Kind::End
        ]
    );
}

#[test]
fn characters_without_a_rule_are_unrecognized() {
    // Neither `!` nor a carriage return has a rule in `scanner.l`.
    assert_eq!(kinds("!"), vec![Kind::Unrecognized, Kind::End]);
    assert_eq!(kinds("\r"), vec![Kind::Unrecognized, Kind::End]);
    assert_eq!(kinds("$"), vec![Kind::Unrecognized, Kind::End]);
}

// -------------------------------------------------------------- statements

#[test]
fn empty_files_are_valid() {
    assert!(unit("").statements.is_empty());
    assert!(unit("\n\n// only a comment\n").statements.is_empty());
}

#[test]
fn modules_and_decorators_are_consumed() {
    // `module { @name: "x" }` produces no IR at all.
    assert!(unit("module {\n  @name: \"x\"\n  @author: \"y\"\n}\n").statements.is_empty());
    // Decorators before a definition are consumed as well.
    let parsed = unit("@doc: \"a\" \"b\"\npublic node foo {\n    input x;\n}\n");
    assert_eq!(definition(&parsed, 0).name.data, "foo");
}

#[test]
fn imports_carry_visibility_short_names_and_paths() {
    let parsed = unit(
        "import \"a.mr\"\n\
         public import \"b.mr\" as b\n\
         private import c\n\
         import d as e\n",
    );

    let imports: Vec<&ImportStatement> = parsed
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Import(import) => Some(import),
            _ => None,
        })
        .collect();

    assert_eq!(imports.len(), 4);
    assert_eq!(imports[0].lib_name.data, "a.mr");
    assert_eq!(imports[0].visibility, Visibility::Default);
    assert!(imports[0].short_name.is_none());

    assert_eq!(imports[1].visibility, Visibility::Public);
    assert_eq!(imports[1].short_name.as_ref().unwrap().data, "b");

    assert_eq!(imports[2].visibility, Visibility::Private);
    // `IMPORT LABEL` uses the label as its own short name.
    assert_eq!(imports[2].lib_name.data, "c");
    assert_eq!(imports[2].short_name.as_ref().unwrap().data, "c");

    assert_eq!(imports[3].short_name.as_ref().unwrap().data, "e");
}

#[test]
fn definitions_record_every_port_field() {
    let parsed = unit(
        "public node foo => __builtin { \n\
             @doc: \"x\"\n\
             input a: 1;\n\
             alias output b [engine_channel];\n\
             modify c;\n\
             toggle d: true;\n\
             input e [ns::type]: 2 + 3;\n\
         }\n",
    );

    let definition = definition(&parsed, 0);
    assert_eq!(definition.name.data, "foo");
    assert_eq!(definition.visibility, Visibility::Public);
    assert!(definition.defines_builtin);
    assert_eq!(definition.builtin_name.data, "__builtin");
    assert!(!definition.is_inline);
    assert_eq!(definition.scope_token.as_ref().unwrap().data, "public");

    let ports = &definition.ports;
    assert_eq!(ports.len(), 5);

    assert_eq!(ports[0].direction, Direction::Input);
    assert_eq!(ports[0].name.data, "a");
    assert!(!ports[0].is_alias);
    assert_eq!(ports[0].default_value, Some(Value::Int { value: 1 }));

    assert_eq!(ports[1].direction, Direction::Output);
    assert!(ports[1].is_alias);
    assert_eq!(ports[1].alias_token.data, "alias");
    let (library, type_name) = ports[1].type_info.as_ref().unwrap();
    assert!(!library.specified);
    assert_eq!(type_name.data, "engine_channel");

    assert_eq!(ports[2].direction, Direction::Modify);
    assert_eq!(ports[3].direction, Direction::Toggle);
    assert_eq!(ports[3].default_value, Some(Value::Bool { value: true }));

    let (library, type_name) = ports[4].type_info.as_ref().unwrap();
    assert_eq!(library.data, "ns");
    assert!(library.specified);
    assert_eq!(type_name.data, "type");

    assert!(definition.body.is_none());
}

#[test]
fn inline_and_auto_definitions() {
    let parsed = unit("inline node a { }\nauto node b { input x; }\n");

    assert!(definition(&parsed, 0).is_inline);

    // `auto` instantiates the node before the definition is registered; the
    // statement records the definition and the flag is what tells the builder
    // to create the instance first.
    let statements = &parsed.statements;
    assert_eq!(statements.len(), 2);
    match &statements[1] {
        Statement::AutoDefinition(definition) => {
            assert_eq!(definition.name.data, "b");
            assert!(definition.ports[0].name.data == "x");
        }
        other => panic!("expected an auto definition, got {:?}", other),
    }
    assert!(nodes(&parsed).is_empty());
}

#[test]
fn body_node_lists() {
    let parsed = unit("node a {\n    input x;\n    b(1)\n    c.d(2)\n}\n");

    let body = definition(&parsed, 0).body.as_ref().unwrap();
    assert_eq!(body.len(), 2);

    assert_eq!(body[0].type_name.data, "b");
    assert!(!body[0].name.specified);
    assert_eq!(body[0].attributes[0].value, Value::Int { value: 1 });

    assert_eq!(body[1].type_name.data, "d");
    // `c.d(2)` carries `c` as the trailing `this` attribute.
    assert_eq!(body[1].attributes.len(), 2);
    assert_eq!(body[1].attributes[1].name.data, "this");
    assert_eq!(
        body[1].attributes[1].value,
        Value::Label { token: Token::with_location("c", 4, 5) }
    );
}

#[test]
fn named_and_unnamed_nodes_at_statement_level() {
    let parsed = unit("wires w()\nmain()\nc0.add_rod_journal(rj0)\n");

    let nodes = nodes(&parsed);
    assert_eq!(nodes.len(), 3);

    assert_eq!(nodes[0].type_name.data, "wires");
    assert_eq!(nodes[0].name.data, "w");

    assert_eq!(nodes[1].type_name.data, "main");
    assert!(!nodes[1].name.specified);

    assert_eq!(nodes[2].type_name.data, "add_rod_journal");
    assert_eq!(nodes[2].attributes[1].name.data, "this");
}

// ------------------------------------------------------------------- values

#[test]
fn arithmetic_precedence_matches_the_grammar() {
    let parsed = unit("node a { input x: 1 + 2 * 3 - 4 / 5; }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    // ((1 + (2 * 3)) - (4 / 5))
    match value {
        Value::Binary { op: BinOp::Sub, left, right, .. } => {
            assert_eq!(*right, binary(BinOp::Div, Value::Int { value: 4 }, Value::Int { value: 5 }));
            match *left {
                Value::Binary { op: BinOp::Add, left: inner_left, right: inner_right, .. } => {
                    assert_eq!(*inner_left, Value::Int { value: 1 });
                    assert_eq!(
                        *inner_right,
                        binary(BinOp::Mul, Value::Int { value: 2 }, Value::Int { value: 3 })
                    );
                }
                other => panic!("expected an addition, got {:?}", other),
            }
        }
        other => panic!("expected a subtraction, got {:?}", other),
    }
}

fn unary(op: UnOp, operand: Value) -> Value {
    let symbol = match op {
        UnOp::Default => "default",
        UnOp::NumericNegate => "-",
        UnOp::BoolNegate => "!",
        UnOp::Positive => "+",
    };

    Value::Unary { token: Token::new(symbol), op, operand: Box::new(operand) }
}

fn binary(op: BinOp, left: Value, right: Value) -> Value {
    let symbol = match op {
        BinOp::Dot => ".",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
    };

    Value::Binary {
        token: Token::new(symbol),
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

#[test]
fn unary_operators_bind_tighter_than_multiplication() {
    let parsed = unit("node a { input x: -1 * 2 + +3; }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    match value {
        Value::Binary { op: BinOp::Add, left, right, .. } => {
            assert_eq!(
                *left,
                binary(
                    BinOp::Mul,
                    unary(UnOp::NumericNegate, Value::Int { value: 1 }),
                    Value::Int { value: 2 }
                )
            );
            assert_eq!(
                *right,
                unary(UnOp::Positive, Value::Int { value: 3 })
            );
        }
        other => panic!("expected an addition, got {:?}", other),
    }
}

#[test]
fn parenthesised_expressions_override_precedence() {
    let parsed = unit("node a { input x: (1 + 2) * 3; }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    assert_eq!(
        value,
        binary(
            BinOp::Mul,
            binary(BinOp::Add, Value::Int { value: 1 }, Value::Int { value: 2 }),
            Value::Int { value: 3 }
        )
    );
}

#[test]
fn dot_chains_nest_left_to_right() {
    let parsed = unit("node a { input x: a.b.c; }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    match value {
        Value::Binary { op: BinOp::Dot, left, right, .. } => {
            assert_eq!(*right, Value::Label { token: Token::with_location("c", 1, 23) });
            assert_eq!(
                *left,
                binary(
                    BinOp::Dot,
                    Value::Label { token: Token::with_location("a", 1, 19) },
                    Value::Label { token: Token::with_location("b", 1, 21) }
                )
            );
        }
        other => panic!("expected a dot operator, got {:?}", other),
    }
}

#[test]
fn method_chains_build_nested_node_references() {
    let parsed = unit("node a { input x: f(1).g(2).h(3); }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    match value {
        Value::NodeRef { node } => {
            assert_eq!(node.type_name.data, "h");
            match &node.attributes[1].value {
                Value::NodeRef { node: inner } => {
                    assert_eq!(inner.type_name.data, "g");
                    match &inner.attributes[1].value {
                        Value::NodeRef { node: innermost } => {
                            assert_eq!(innermost.type_name.data, "f");
                        }
                        other => panic!("expected a node reference, got {:?}", other),
                    }
                }
                other => panic!("expected a node reference, got {:?}", other),
            }
        }
        other => panic!("expected a node reference, got {:?}", other),
    }
}

#[test]
fn adjacent_string_literals_concatenate() {
    let parsed = unit("node a { input x: \"ab\" \"cd\"; }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    assert_eq!(value, Value::Str { value: "abcd".to_string() });
}

#[test]
fn attributes_can_be_named_or_positional() {
    let parsed = unit("node a { input x: f(1, named: 2); }\n");
    let value = definition(&parsed, 0).ports[0].default_value.clone().unwrap();

    match value {
        Value::NodeRef { node } => {
            assert!(!node.attributes[0].name.specified);
            assert_eq!(node.attributes[0].value, Value::Int { value: 1 });
            assert_eq!(node.attributes[1].name.data, "named");
            assert_eq!(node.attributes[1].value, Value::Int { value: 2 });
        }
        other => panic!("expected a node reference, got {:?}", other),
    }
}

#[test]
fn qualified_type_names() {
    let parsed = unit("node a { input x: ns::type(1); input y: ::type(2); }\n");
    let ports = &definition(&parsed, 0).ports;

    match &ports[0].default_value {
        Some(Value::NodeRef { node }) => {
            assert_eq!(node.library.data, "ns");
            assert_eq!(node.type_name.data, "type");
        }
        other => panic!("expected a node reference, got {:?}", other),
    }

    match &ports[1].default_value {
        Some(Value::NodeRef { node }) => {
            // `::type` names an empty, but specified, library.
            assert_eq!(node.library.data, "");
            assert!(node.library.specified);
        }
        other => panic!("expected a node reference, got {:?}", other),
    }
}

// ------------------------------------------------------------- rejections

#[test]
fn malformed_scripts_are_rejected() {
    // Unbalanced parentheses.
    assert!(parse_unit("test.mr", "a(1").is_err());
    // Missing semicolon in a port list.
    assert!(parse_unit("test.mr", "node a { input x }").is_err());
    // A node definition needs a name.
    assert!(parse_unit("test.mr", "node { }").is_err());
    // A bare value is not a statement.
    assert!(parse_unit("test.mr", "1").is_err());
    assert!(parse_unit("test.mr", "a.b").is_err());
    // Unrecognised characters.
    assert!(parse_unit("test.mr", "a!(1)").is_err());
    // Unterminated string.
    assert!(parse_unit("test.mr", "node a { input x: \"unterminated; }").is_err());
    // Expressions cannot be empty.
    assert!(parse_unit("test.mr", "node a { input x: ; }").is_err());
    // A port needs a direction.
    assert!(parse_unit("test.mr", "node a { x; }").is_err());
}

#[test]
fn errors_carry_a_position() {
    let failure = error("node a {\n    input x\n}\n");
    assert_eq!(failure.line, 3);
}
