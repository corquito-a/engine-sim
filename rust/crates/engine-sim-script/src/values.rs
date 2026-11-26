//! What the graph computes: `NodeOutput::fullCompute`, and nothing else.
//!
//! piranha's value layer is pull-based. `AddOperationNodeOutput::fullCompute`
//! recurses into its input connections, `LiteralNodeOutput::fullCompute` copies
//! the data the literal node injected, and neither caches anything -- the
//! builtin `_evaluate`s are empty. A value is therefore a *function of the
//! wiring*, and `NodeProgram::execute` only matters for the stateful classes
//! `es_script` adds. That is why this module is a read-only walk of [`Graph`]:
//! no program, no evaluation order, no mutation.
//!
//! Everything here keys on the class *name* after the rules prefix
//! ([`crate::graph::class_name`]), because both rule sets register their own
//! `float_multiply`: `__engine_sim__float_multiply` is a
//! `MultiplyOperationNodeOutput<float>` exactly as `__piranha__float_multiply`
//! is.
//!
//! What is deliberately *not* here yet: the vector channels, the engine's own
//! classes, and anything whose value needs `initialize` (a `FunctionChannel` is
//! a sampler, not a value). Those print their channel name with no text, which
//! is what `script_reference --values` prints for them too.

use engine_sim_core::gas::GasSystem;

use crate::expansion::cxx_float_text;
use crate::graph::{class_name, Graph, NodeClass, PortId};
use crate::lower::Literal;

/// A fundamental channel's value, the four types `piranha::native_*` covers.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Float(f64),
    Int(i32),
    Bool(bool),
    Text(String),
}

/// The channel a value lives in, as far as the value layer is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Float,
    Int,
    Bool,
    Text,
}

impl Kind {
    pub fn of_channel(channel_type: &str) -> Option<Kind> {
        Some(match channel_type {
            "FloatNodeType" => Kind::Float,
            "IntNodeType" => Kind::Int,
            "BoolNodeType" => Kind::Bool,
            "StringNodeType" => Kind::Text,
            _ => return None,
        })
    }

    /// The word the reference prints for a fundamental channel: it compares
    /// each output against `FundamentalType::*Type` and spells the match out,
    /// so a `FloatNodeType` port reads `float`, not `FloatNodeType`.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Float => "float",
            Kind::Int => "int",
            Kind::Bool => "bool",
            Kind::Text => "string",
        }
    }
}

impl Value {
    pub fn kind(&self) -> Kind {
        match self {
            Value::Float(_) => Kind::Float,
            Value::Int(_) => Kind::Int,
            Value::Bool(_) => Kind::Bool,
            Value::Text(_) => Kind::Text,
        }
    }

    pub fn float(&self) -> Option<f64> {
        match self {
            Value::Float(value) => Some(*value),
            Value::Int(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i32> {
        match self {
            Value::Int(value) => Some(*value),
            Value::Float(value) => Some(*value as i32),
            _ => None,
        }
    }

    /// `std::ostream`'s spelling of the value: what the reference prints, and
    /// what `float_to_string` produces.
    pub fn text(&self) -> String {
        match self {
            Value::Float(value) => cxx_float_text(*value),
            Value::Int(value) => value.to_string(),
            Value::Bool(value) => if *value { "1" } else { "0" }.to_string(),
            Value::Text(value) => value.clone(),
        }
    }
}

/// `std::stringstream >> double`: the longest parseable prefix, and `0.0` when
/// there is none. `num_get` takes leading whitespace, a sign, digits with at
/// most one point and a well-formed exponent; whatever follows the number is
/// left unread.
pub fn cxx_parse_float(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut index = 0;

    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let start = index;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        index += 1;
    }

    let mut digits = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
        digits += 1;
    }

    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
            digits += 1;
        }
    }

    if digits == 0 {
        return 0.0;
    }

    // An exponent only counts when it is well formed; otherwise the number
    // ends where the mantissa's digits do.
    let mantissa_end = index;
    if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        let mut exponent = index + 1;
        if exponent < bytes.len() && (bytes[exponent] == b'+' || bytes[exponent] == b'-') {
            exponent += 1;
        }

        let digits_start = exponent;
        while exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
            exponent += 1;
        }

        index = if exponent > digits_start { exponent } else { mantissa_end };
    }

    text[start..index].parse().unwrap_or(0.0)
}

impl Graph {
    /// `NodeOutput::fullCompute` on one port.
    pub fn value_of(&self, port: PortId) -> Option<Value> {
        self.value_of_guarded(port, 0)
    }

    /// Depth-bounded so a malformed graph cannot recurse forever: `build`
    /// cannot produce a cycle, but this walk should never hang.
    fn value_of_guarded(&self, port: PortId, depth: usize) -> Option<Value> {
        if depth > 256 {
            return None;
        }

        let owner = self.port(port).owner?;
        self.node_value(owner, depth + 1)
    }

    fn input_value(&self, node: usize, name: &str, depth: usize) -> Option<Value> {
        let connection = self
            .node(node)
            .inputs
            .iter()
            .find(|input| input.name == name)?
            .connection?;

        self.value_of_guarded(connection, depth)
    }

    fn binary_inputs(&self, node: usize, depth: usize) -> Option<(Value, Value)> {
        Some((
            self.input_value(node, "__in0", depth)?,
            self.input_value(node, "__in1", depth)?,
        ))
    }

    fn node_value(&self, node: usize, depth: usize) -> Option<Value> {
        // What the node's `_evaluate` wrote into its own output, if anything has
        // run: `fullCompute` reads an output's data, and `console_in`'s data comes
        // from standard input rather than from the graph.
        if let Some(value) = self.outputs.get(&node) {
            return Some(value.clone());
        }

        let data = self.node(node);

        match data.class {
            // `IrLiteralNode<T>::_generateNode` injected the value and
            // `DefaultLiteralNode<T>::_evaluate` copies it into the output.
            NodeClass::Literal(_) => match data.literal.as_ref()? {
                Literal::Float(value) => Some(Value::Float(*value)),
                Literal::Int(value) => Some(Value::Int(*value)),
                Literal::Bool(value) => Some(Value::Bool(*value)),
                Literal::Str(value) => Some(Value::Text(value.clone())),
            },

            // A channel node registers no ports; a container's outputs are
            // references, which `dump` resolves on its own.
            NodeClass::Channel | NodeClass::Container => None,

            NodeClass::Builtin => {
                let class = class_name(&data.builtin_name)?;

                match class {
                    "float_add" | "int_add" | "string_add" => {
                        let (left, right) = self.binary_inputs(node, depth)?;
                        Some(match (left, right) {
                            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
                            (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_add(b)),
                            (Value::Text(a), Value::Text(b)) => Value::Text(a + &b),
                            _ => return None,
                        })
                    }

                    "float_subtract" | "int_subtract" => {
                        let (left, right) = self.binary_inputs(node, depth)?;
                        Some(match (left, right) {
                            (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
                            (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_sub(b)),
                            _ => return None,
                        })
                    }

                    "float_multiply" | "int_multiply" => {
                        let (left, right) = self.binary_inputs(node, depth)?;
                        Some(match (left, right) {
                            (Value::Float(a), Value::Float(b)) => Value::Float(a * b),
                            (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_mul(b)),
                            _ => return None,
                        })
                    }

                    "float_divide" | "int_divide" => {
                        let (left, right) = self.binary_inputs(node, depth)?;
                        Some(match (left, right) {
                            (Value::Float(a), Value::Float(b)) => Value::Float(a / b),
                            // Integer division by zero is undefined in C++; the
                            // reference faults rather than producing a value.
                            (Value::Int(a), Value::Int(b)) => Value::Int(a.checked_div(b)?),
                            _ => return None,
                        })
                    }

                    "float_negate" | "int_negate" => {
                        Some(match self.input_value(node, "__in", depth)? {
                            Value::Float(value) => Value::Float(-value),
                            Value::Int(value) => Value::Int(value.wrapping_neg()),
                            _ => return None,
                        })
                    }

                    "bool_negate" => Some(match self.input_value(node, "__in", depth)? {
                        Value::Bool(value) => Value::Bool(!value),
                        _ => return None,
                    }),

                    // `NoOpNode` registers `__out` as an output *reference* to
                    // `__in`, so the value is the input's.
                    "float_positive" | "int_positive" => self.input_value(node, "__in", depth),

                    "int_to_float" => {
                        Some(Value::Float(self.input_value(node, "__in", depth)?.int()? as f64))
                    }

                    "string_to_float" => match self.input_value(node, "__in", depth)? {
                        Value::Text(text) => Some(Value::Float(cxx_parse_float(&text))),
                        _ => None,
                    },

                    "float_to_string" => Some(Value::Text(
                        cxx_float_text(self.input_value(node, "__in", depth)?.float()?),
                    )),

                    "int_to_string" => Some(Value::Text(
                        self.input_value(node, "__in", depth)?.int()?.to_string(),
                    )),

                    "string_to_int" => match self.input_value(node, "__in", depth)? {
                        Value::Text(text) => Some(Value::Int(cxx_parse_float(&text) as i32)),
                        _ => None,
                    },

                    // `es_script`'s two flow constants: a `flow` input and a
                    // float output computed by `GasSystem`.
                    "k_28inH2O" | "k_carb" => {
                        let flow = self.input_value(node, "flow", depth)?.float()?;
                        Some(Value::Float(match class {
                            "k_28inH2O" => GasSystem::k_28in_h2o(flow),
                            _ => GasSystem::k_carb(flow),
                        }))
                    }

                    // Vectors, the engine's own classes, and anything else whose
                    // value needs `initialize`.
                    _ => None,
                }
            }
        }
    }
}

/// The records `script_reference --values` prints, in the reference's order and
/// with [`crate::graph::dump`]'s node numbering.
pub fn dump(graph: &Graph) -> Vec<String> {
    let mut out = Vec::new();

    let Some(root) = graph.root else {
        return out;
    };

    let (_, ids) = crate::graph::assign_ids(graph, root);
    dump_container(graph, root, &ids, &mut out);
    out
}

fn dump_container(graph: &Graph, container: usize, ids: &[Option<usize>], out: &mut Vec<String>) {
    for node in &graph.node(container).nodes {
        let Some(id) = ids[*node] else {
            continue;
        };

        for output in &graph.node(*node).outputs {
            emit(graph, id, &output.name, Some(output.port), out);
        }

        // `m_outputReferences`, the null ones included: an unresolved container
        // output is a `-,-` record, not a missing line.
        for reference in &graph.node(*node).output_references {
            emit(graph, id, &reference.name, reference.port, out);
        }
    }

    for child in &graph.node(container).children {
        dump_container(graph, *child, ids, out);
    }
}

fn emit(graph: &Graph, id: usize, name: &str, port: Option<PortId>, out: &mut Vec<String>) {
    let Some(port) = port else {
        out.push(format!("V,{id},{name},-,-"));
        return;
    };

    let channel = graph.port(port).channel_type;
    let value = graph.value_of(port);

    match Kind::of_channel(channel) {
        Some(kind) => match value {
            Some(value) if value.kind() == kind => {
                out.push(format!("V,{id},{name},{},{}", kind.label(), value.text()));
            }
            // Connected but not computable: the port is a channel the value
            // layer does not model, so only its type is known.
            _ => out.push(format!("V,{id},{name},{},-", kind.label())),
        },
        None => out.push(format!("V,{id},{name},{channel},-")),
    }
}
