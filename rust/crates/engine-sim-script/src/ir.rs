//! The intermediate representation produced by the `.mr` front end.
//!
//! The shapes mirror the piranha classes one for one (`IrNode`, `IrAttribute`,
//! `IrAttributeDefinition`, `IrNodeDefinition`, `IrImportStatement`,
//! `IrValue*`), because the reference oracle and the later milestones consume
//! exactly that structure. Names match the C++ ones so the two can be diffed
//! field by field.

/// `piranha::IrTokenInfo`: a token plus its source location and the
/// `specified` / `empty` flags the original uses to tell "the author wrote
/// nothing here" from "the author wrote an empty string".
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Token {
    pub data: String,
    /// False for default-constructed tokens (`IrTokenInfo()`).
    pub specified: bool,
    /// True for default-constructed tokens (`IrTokenInfo()`).
    pub empty: bool,
    pub line: u32,
    pub col: u32,
}

impl Token {
    /// A token the parser synthesised, equivalent to `IrTokenInfo_string(data)`
    /// followed by the flags the grammar sets (line/column are not used by the
    /// IR consumers, so they stay zero).
    pub fn new(data: impl Into<String>) -> Self {
        Token { data: data.into(), specified: true, empty: false, line: 0, col: 0 }
    }

    pub fn with_location(data: impl Into<String>, line: u32, col: u32) -> Self {
        Token { data: data.into(), specified: true, empty: false, line, col }
    }

    /// `IrTokenInfo_string name; name.specified = false;` (unnamed nodes).
    pub fn unspecified() -> Self {
        Token::default()
    }

    pub fn is_specified(&self) -> bool {
        self.specified
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Dot,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Default,
    NumericNegate,
    BoolNegate,
    Positive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
    Modify,
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Public,
    Default,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Public => "public",
            Visibility::Default => "default",
        }
    }
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Input => "input",
            Direction::Output => "output",
            Direction::Modify => "modify",
            Direction::Toggle => "toggle",
        }
    }
}

/// `piranha::IrValue` and its subclasses.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int { value: i32 },
    Float { value: f64 },
    Bool { value: bool },
    Str { value: String },
    Label { token: Token },
    NodeRef { node: Box<Node> },
    Binary { token: Token, op: BinOp, left: Box<Value>, right: Box<Value> },
    Unary { token: Token, op: UnOp, operand: Box<Value> },
}

/// `piranha::IrAttribute`: a named or positional argument of a node.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: Token,
    pub value: Value,
}

/// `piranha::IrAttributeList`, materialised as a vector in source order.
pub type AttributeList = Vec<Attribute>;

/// `piranha::IrNode`.
///
/// `IrNode` can also point at the `IrNodeDefinition` that created it (used by
/// `auto node ...`, which instantiates a node that bypasses name lookup). That
/// is modelled by [`Statement::AutoDefinition`] instead of a back pointer, so a
/// definition is never owned twice.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// `type_name_namespace.data[1]`
    pub type_name: Token,
    /// `type_name_namespace.data[0]`: the namespace, if any.
    pub library: Token,
    /// Empty and unspecified for anonymous nodes.
    pub name: Token,
    pub attributes: AttributeList,
}

impl Node {
    pub fn new(type_name: Token, library: Token, name: Token, attributes: AttributeList) -> Self {
        Node { type_name, library, name, attributes }
    }

    /// `IrNode::setThis`: `data_access '.' node` is modelled by the original as
    /// a trailing attribute literally named `this`, not as a separate field, so
    /// that the definition's `input this` port resolves it like any other
    /// named argument.
    pub fn set_this(&mut self, value: Value) {
        self.attributes.push(Attribute { name: Token::new("this"), value });
    }
}

/// `piranha::IrAttributeDefinition`: one entry of a node definition's port list.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeDefinition {
    pub direction: Direction,
    pub direction_token: Token,
    pub name: Token,
    pub is_alias: bool,
    /// Only meaningful when `is_alias` is set (`getAliasToken()` returns null
    /// otherwise in the original).
    pub alias_token: Token,
    /// `type_name_namespace`: index 0 is the namespace, index 1 the name.
    pub type_info: Option<(Token, Token)>,
    pub default_value: Option<Value>,
}

/// `piranha::IrNodeDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDefinition {
    pub name: Token,
    pub is_inline: bool,
    pub defines_builtin: bool,
    pub builtin_name: Token,
    pub visibility: Visibility,
    /// The `public` / `private` keyword token, when one was written.
    pub scope_token: Option<Token>,
    pub ports: Vec<AttributeDefinition>,
    pub body: Option<Vec<Node>>,
}

impl NodeDefinition {
    pub fn new(name: Token) -> Self {
        NodeDefinition {
            name,
            is_inline: false,
            defines_builtin: false,
            builtin_name: Token::unspecified(),
            visibility: Visibility::Default,
            scope_token: None,
            ports: Vec::new(),
            body: None,
        }
    }
}

/// `piranha::IrImportStatement`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportStatement {
    pub visibility: Visibility,
    pub lib_name: Token,
    pub short_name: Option<Token>,
}

/// A statement of `statement_list` in the order the grammar's actions run.
#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Import(ImportStatement),
    /// `auto node ...`: the parser instantiates a node *before* it registers
    /// the definition, so the statement carries both in that order.
    AutoDefinition(NodeDefinition),
    Definition(NodeDefinition),
    Node(Node),
}

/// `piranha::IrCompilationUnit` restricted to what the parser produces.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// The path exactly as the compiler opened it.
    pub path: String,
    pub statements: Vec<Statement>,
}
