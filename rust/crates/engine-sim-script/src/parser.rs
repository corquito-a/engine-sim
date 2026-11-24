//! Recursive-descent parser for `.mr` scripts.
//!
//! Follows `piranha/flex-bison/specification.y` production for production. The
//! grammar is LALR(1), but every decision point only needs one or two tokens of
//! lookahead, so a hand written parser reproduces it exactly while keeping the
//! precedence ladder (`value > add_exp > mul_exp > unary_exp > data_access >
//! primary_exp > atomic_value`) readable.

use crate::ir::*;
use crate::lexer::{tokenize, Kind, Literal, Token as LexToken};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl ParseError {
    fn new(token: &LexToken, message: impl Into<String>) -> Self {
        ParseError { line: token.line, col: token.col, message: message.into() }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parses a whole source file. `path` is recorded verbatim in the unit.
pub fn parse_unit(path: &str, src: &str) -> Result<Unit, ParseError> {
    let mut parser = Parser::new(src);
    let mut statements = Vec::new();

    while !parser.at(Kind::End) {
        if let Some(statement) = parser.statement()? {
            statements.push(statement);
        }
    }

    Ok(Unit { path: path.to_string(), statements })
}

pub struct Parser {
    tokens: Vec<LexToken>,
    pos: usize,
}

fn ir_token(token: &LexToken) -> Token {
    Token::with_location(token.text.clone(), token.line, token.col)
}

/// `string : STRING | string STRING` -- adjacent literals concatenate.
fn string_token(token: &LexToken) -> Token {
    match &token.literal {
        Literal::Str(text) => Token::with_location(text.clone(), token.line, token.col),
        _ => unreachable!("string_token on a non-string token"),
    }
}

impl Parser {
    pub fn new(src: &str) -> Self {
        Parser { tokens: tokenize(src), pos: 0 }
    }

    fn peek(&self) -> &LexToken {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_kind(&self) -> Kind {
        self.peek().kind
    }

    fn kind_at(&self, offset: usize) -> Kind {
        let index = (self.pos + offset).min(self.tokens.len() - 1);
        self.tokens[index].kind
    }

    fn at(&self, kind: Kind) -> bool {
        self.peek_kind() == kind
    }

    fn bump(&mut self) -> LexToken {
        let token = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        token
    }

    fn expect(&mut self, kind: Kind, what: &str) -> Result<LexToken, ParseError> {
        if self.at(kind) {
            Ok(self.bump())
        }
        else {
            Err(ParseError::new(self.peek(), format!("expected {}", what)))
        }
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError::new(self.peek(), message))
    }

    // ------------------------------------------------------------- statements

    fn statement(&mut self) -> Result<Option<Statement>, ParseError> {
        match self.peek_kind() {
            Kind::Module => {
                // MODULE '{' decorator_list '}'
                self.bump();
                self.expect(Kind::LBrace, "'{'")?;
                self.decorator_list()?;
                self.expect(Kind::RBrace, "'}'")?;
                Ok(None)
            }
            Kind::Import => Ok(Some(Statement::Import(self.import_statement()?))),
            Kind::Public | Kind::Private => {
                if self.kind_at(1) == Kind::Import {
                    Ok(Some(Statement::Import(self.import_statement()?)))
                }
                else {
                    self.node_decorator()
                }
            }
            // A `type_name` can be `OPERATOR '+'`, so `operator` only starts a
            // definition after the `node` keyword; on its own it is a node.
            Kind::Decorator | Kind::Auto | Kind::Inline | Kind::Node => self.node_decorator(),
            Kind::End => Ok(None),
            _ => Ok(Some(Statement::Node(self.node_member()?))),
        }
    }

    /// `node_decorator : decorator_list immediate_node_definition | immediate_node_definition`
    fn node_decorator(&mut self) -> Result<Option<Statement>, ParseError> {
        if self.at(Kind::Decorator) {
            self.decorator_list()?;
        }

        let is_auto = if self.at(Kind::Auto) {
            self.bump();
            true
        }
        else {
            false
        };

        let scope = match self.peek_kind() {
            Kind::Public | Kind::Private => Some(self.bump()),
            _ => None,
        };

        let mut definition = self.node_definition()?;

        if let Some(token) = scope {
            definition.visibility = if token.kind == Kind::Public {
                Visibility::Public
            }
            else {
                Visibility::Private
            };
            definition.scope_token = Some(ir_token(&token));
        }

        Ok(Some(if is_auto {
            Statement::AutoDefinition(definition)
        }
        else {
            Statement::Definition(definition)
        }))
    }

    /// `decorator : DECORATOR LABEL ':' string`
    fn decorator_list(&mut self) -> Result<(), ParseError> {
        let mut count = 0;
        while self.at(Kind::Decorator) {
            self.bump();
            self.expect(Kind::Label, "a decorator name")?;
            self.expect(Kind::Colon, "':'")?;
            self.string()?;
            count += 1;
        }

        if count == 0 {
            return self.error("expected a decorator");
        }

        Ok(())
    }

    fn string(&mut self) -> Result<Token, ParseError> {
        let first = self.expect(Kind::Str, "a string literal")?;
        let mut token = string_token(&first);

        while self.at(Kind::Str) {
            let next = self.bump();
            let next = string_token(&next);
            token.data.push_str(&next.data);
        }

        Ok(token)
    }

    /// `import_statement_short_name`
    fn import_statement(&mut self) -> Result<ImportStatement, ParseError> {
        let visibility = match self.peek_kind() {
            Kind::Public => {
                self.bump();
                Visibility::Public
            }
            Kind::Private => {
                self.bump();
                Visibility::Private
            }
            _ => Visibility::Default,
        };

        self.expect(Kind::Import, "'import'")?;

        let mut short_name = None;
        let lib_name = match self.peek_kind() {
            Kind::Str => self.string()?,
            Kind::Label => {
                // `IMPORT LABEL`: the label doubles as the short name.
                let token = self.bump();
                let token = ir_token(&token);
                short_name = Some(token.clone());
                token
            }
            _ => return self.error("expected a path or a label after 'import'"),
        };

        if self.at(Kind::As) {
            self.bump();
            let token = self.expect(Kind::Label, "a label after 'as'")?;
            short_name = Some(ir_token(&token));
        }

        Ok(ImportStatement { visibility, lib_name, short_name })
    }

    // ---------------------------------------------------- node definitions

    /// `type_name : LABEL | OPERATOR standard_operator`
    fn type_name(&mut self) -> Result<Token, ParseError> {
        match self.peek_kind() {
            Kind::Label => {
                let token = self.bump();
                Ok(ir_token(&token))
            }
            Kind::Operator => {
                let keyword = self.bump();
                let operator = self.standard_operator()?;
                // `info.data = std::string("operator") + $2.data`
                Ok(Token::with_location(
                    format!("operator{}", operator.text),
                    keyword.line,
                    keyword.col,
                ))
            }
            _ => self.error("expected a type name"),
        }
    }

    fn standard_operator(&mut self) -> Result<LexToken, ParseError> {
        match self.peek_kind() {
            Kind::Minus | Kind::Plus | Kind::Slash | Kind::Star => Ok(self.bump()),
            _ => self.error("expected an operator symbol"),
        }
    }

    /// `type_name_namespace` -- returns `(namespace, name)`.
    fn type_name_namespace(&mut self) -> Result<(Token, Token), ParseError> {
        if self.at(Kind::NamespacePointer) {
            self.bump();
            let name = self.type_name()?;
            // `set.data[0].specified = true;` with an empty name.
            let mut library = Token::unspecified();
            library.specified = true;
            return Ok((library, name));
        }

        let first = self.type_name()?;

        if self.at(Kind::NamespacePointer) {
            self.bump();
            let name = self.type_name()?;
            return Ok((first, name));
        }

        Ok((Token::unspecified(), first))
    }

    /// `node : type_name_namespace LABEL? connection_block`
    fn node(&mut self) -> Result<Node, ParseError> {
        let (library, type_name) = self.type_name_namespace()?;

        let name = if self.at(Kind::Label) {
            let token = self.bump();
            ir_token(&token)
        }
        else {
            // `name.specified = false; name.data = "";`
            Token::unspecified()
        };

        let attributes = self.connection_block()?;

        Ok(Node::new(type_name, library, name, attributes))
    }

    /// `node_member : node | data_access '.' node`
    ///
    /// `node` is `type_name_namespace LABEL? connection_block`, so the named
    /// form (`wires wires()`) has to be recognised before the value-expression
    /// path is tried -- but only when no `.` follows, because `f(1).g(2)` is
    /// the `data_access '.' node` production.
    fn node_member(&mut self) -> Result<Node, ParseError> {
        let checkpoint = self.pos;

        if let Some(node) = self.try_node()? {
            if !self.at(Kind::Dot) {
                return Ok(node);
            }
        }

        self.pos = checkpoint;

        let value = self.data_access()?;

        if self.at(Kind::Dot) {
            self.bump();
            let mut node = self.node()?;
            node.set_this(value);
            return Ok(node);
        }

        match value {
            Value::NodeRef { node } => Ok(*node),
            _ => self.error("expected a node"),
        }
    }

    /// Speculatively parses the `node` production; rewinds and returns `None`
    /// when the statement is not a node of that shape.
    fn try_node(&mut self) -> Result<Option<Node>, ParseError> {
        let checkpoint = self.pos;

        let Ok((library, type_name)) = self.type_name_namespace() else {
            self.pos = checkpoint;
            return Ok(None);
        };

        let name = if self.at(Kind::Label) {
            ir_token(&self.bump())
        }
        else {
            Token::unspecified()
        };

        if !self.at(Kind::LParen) {
            self.pos = checkpoint;
            return Ok(None);
        }

        let attributes = self.connection_block()?;

        Ok(Some(Node::new(type_name, library, name, attributes)))
    }

    /// `connection_block : '(' ')' | '(' attribute_list ')'`
    fn connection_block(&mut self) -> Result<AttributeList, ParseError> {
        self.expect(Kind::LParen, "'('")?;

        let mut attributes = Vec::new();
        if self.at(Kind::RParen) {
            self.bump();
            return Ok(attributes);
        }

        loop {
            attributes.push(self.attribute()?);
            if self.at(Kind::Comma) {
                self.bump();
                continue;
            }
            break;
        }

        self.expect(Kind::RParen, "')'")?;

        Ok(attributes)
    }

    /// `attribute : LABEL ':' value | value`
    fn attribute(&mut self) -> Result<Attribute, ParseError> {
        if self.at(Kind::Label) && self.kind_at(1) == Kind::Colon {
            let name = self.bump();
            let name = ir_token(&name);
            self.bump(); // ':'
            let value = self.value()?;
            return Ok(Attribute { name, value });
        }

        let value = self.value()?;

        Ok(Attribute { name: Token::unspecified(), value })
    }

    /// `node_definition` (+ the optional `INLINE` / shadowing suffixes).
    fn node_definition(&mut self) -> Result<NodeDefinition, ParseError> {
        let is_inline = if self.at(Kind::Inline) {
            self.bump();
            true
        }
        else {
            false
        };

        self.expect(Kind::Node, "'node'")?;

        let name = match self.peek_kind() {
            Kind::Label => {
                let token = self.bump();
                ir_token(&token)
            }
            Kind::Operator => {
                let keyword = self.bump();
                let operator = self.standard_operator()?;
                Token::with_location(
                    format!("operator{}", operator.text),
                    keyword.line,
                    keyword.col,
                )
            }
            _ => return self.error("expected a node name"),
        };

        let mut definition = NodeDefinition::new(name);
        definition.is_inline = is_inline;

        // `node_inline BUILTIN_POINTER LABEL`
        if self.at(Kind::BuiltinPointer) {
            self.bump();
            let token = self.expect(Kind::Label, "a built-in node name")?;
            definition.builtin_name = ir_token(&token);
            definition.defines_builtin = true;
        }

        definition.ports = self.port_definitions()?;

        if self.at(Kind::RBrace) {
            self.bump();
            definition.body = None;
        }
        else {
            definition.body = Some(self.node_list()?);
            self.expect(Kind::RBrace, "'}'")?;
        }

        Ok(definition)
    }

    /// `port_definitions : '{' (documented_port_definition ';')*`
    ///
    /// Leaves the closing `}` to the caller, exactly like the grammar (it
    /// belongs to `node_definition`).
    fn port_definitions(&mut self) -> Result<Vec<AttributeDefinition>, ParseError> {
        self.expect(Kind::LBrace, "'{'")?;

        // Only a decorator or a port declaration can continue the list; any
        // other token (a label, say) starts the node body, which is where the
        // LALR parser reduces `port_definitions` too.
        let mut ports = Vec::new();
        while matches!(
            self.peek_kind(),
            Kind::Decorator | Kind::Alias | Kind::Input | Kind::Output | Kind::Modify | Kind::Toggle
        ) {
            if self.at(Kind::Decorator) {
                self.decorator_list()?;
            }

            ports.push(self.port_connection()?);
            self.expect(Kind::Semi, "';'")?;
        }

        Ok(ports)
    }

    /// `port_connection : port_value [':' value]`
    fn port_connection(&mut self) -> Result<AttributeDefinition, ParseError> {
        let alias_token = if self.at(Kind::Alias) {
            Some(self.bump())
        }
        else {
            None
        };

        let direction = match self.peek_kind() {
            Kind::Input => Direction::Input,
            Kind::Output => Direction::Output,
            Kind::Modify => Direction::Modify,
            Kind::Toggle => Direction::Toggle,
            _ => return self.error("expected a port direction"),
        };
        let direction_token = self.bump();

        let name = self.expect(Kind::Label, "a port name")?;

        let mut definition = AttributeDefinition {
            direction,
            direction_token: ir_token(&direction_token),
            name: ir_token(&name),
            is_alias: alias_token.is_some(),
            alias_token: match &alias_token {
                Some(token) => ir_token(token),
                None => Token::unspecified(),
            },
            type_info: None,
            default_value: None,
        };

        // `port_value : port_status '[' type_name_namespace ']'`
        if self.at(Kind::LBracket) {
            self.bump();
            let (library, type_name) = self.type_name_namespace()?;
            self.expect(Kind::RBracket, "']'")?;
            definition.type_info = Some((library, type_name));
        }

        if self.at(Kind::Colon) {
            self.bump();
            definition.default_value = Some(self.value()?);
        }

        Ok(definition)
    }

    /// `node_list : node_member+`
    fn node_list(&mut self) -> Result<Vec<Node>, ParseError> {
        let mut nodes = Vec::new();

        while !self.at(Kind::RBrace) {
            if self.at(Kind::End) {
                return self.error("unexpected end of file inside a node definition");
            }
            nodes.push(self.node_member()?);
        }

        Ok(nodes)
    }

    // ---------------------------------------------------------- expressions

    /// `value : add_exp`
    fn value(&mut self) -> Result<Value, ParseError> {
        self.add_exp()
    }

    fn add_exp(&mut self) -> Result<Value, ParseError> {
        let mut value = self.mul_exp()?;

        loop {
            let op = match self.peek_kind() {
                Kind::Plus => BinOp::Add,
                Kind::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let right = self.mul_exp()?;
            value = Value::Binary { op, left: Box::new(value), right: Box::new(right) };
        }

        Ok(value)
    }

    fn mul_exp(&mut self) -> Result<Value, ParseError> {
        let mut value = self.unary_exp()?;

        loop {
            let op = match self.peek_kind() {
                Kind::Star => BinOp::Mul,
                Kind::Slash => BinOp::Div,
                _ => break,
            };
            self.bump();
            let right = self.unary_exp()?;
            value = Value::Binary { op, left: Box::new(value), right: Box::new(right) };
        }

        Ok(value)
    }

    /// `unary_exp : data_access | '-' data_access | '+' data_access`
    ///
    /// `'!' data_access` exists in `specification.y` but `scanner.l` never
    /// produces `'!'`, so the production is unreachable in the original too.
    fn unary_exp(&mut self) -> Result<Value, ParseError> {
        let op = match self.peek_kind() {
            Kind::Minus => UnOp::NumericNegate,
            Kind::Plus => UnOp::Positive,
            _ => return self.data_access(),
        };

        self.bump();
        let operand = self.data_access()?;

        Ok(Value::Unary { op, operand: Box::new(operand) })
    }

    /// `data_access : primary_exp ('.' label_value | '.' inline_node)*`
    ///
    /// The original builds `data_access '.' inline_node` through
    /// `inline_node_member`, which nests the same way this loop does.
    fn data_access(&mut self) -> Result<Value, ParseError> {
        let mut value = self.primary_exp()?;

        while self.at(Kind::Dot) {
            // What follows the dot decides between a label (`x.y`) and an
            // inline node (`x.y(...)`).
            let starts_node = match self.kind_at(1) {
                Kind::NamespacePointer | Kind::Operator => true,
                Kind::Label => matches!(self.kind_at(2), Kind::LParen | Kind::NamespacePointer),
                _ => false,
            };

            if starts_node {
                self.bump(); // '.'
                let mut node = self.inline_node()?;
                node.set_this(value);
                value = Value::NodeRef { node: Box::new(node) };
                continue;
            }

            if self.kind_at(1) == Kind::Label {
                self.bump(); // '.'
                let label = self.bump();
                value = Value::Binary {
                    op: BinOp::Dot,
                    left: Box::new(value),
                    right: Box::new(Value::Label { token: ir_token(&label) }),
                };
                continue;
            }

            break;
        }

        Ok(value)
    }

    /// `inline_node : type_name_namespace connection_block`
    fn inline_node(&mut self) -> Result<Node, ParseError> {
        let (library, type_name) = self.type_name_namespace()?;
        let attributes = self.connection_block()?;

        Ok(Node::new(type_name, library, Token::unspecified(), attributes))
    }

    /// `primary_exp : atomic_value | '(' value ')'`
    fn primary_exp(&mut self) -> Result<Value, ParseError> {
        if self.at(Kind::LParen) {
            self.bump();
            let value = self.value()?;
            self.expect(Kind::RParen, "')'")?;
            return Ok(value);
        }

        self.atomic_value()
    }

    /// `atomic_value : label_value | inline_node_member | constant`
    fn atomic_value(&mut self) -> Result<Value, ParseError> {
        match self.peek_kind() {
            Kind::Int => {
                let token = self.bump();
                match token.literal {
                    Literal::Int(value) => Ok(Value::Int { value }),
                    _ => unreachable!("INT token without an integer literal"),
                }
            }
            Kind::Float => {
                let token = self.bump();
                match token.literal {
                    Literal::Float(value) => Ok(Value::Float { value }),
                    _ => unreachable!("FLOAT token without a float literal"),
                }
            }
            Kind::Bool => {
                let token = self.bump();
                match token.literal {
                    Literal::Bool(value) => Ok(Value::Bool { value }),
                    _ => unreachable!("BOOL token without a boolean literal"),
                }
            }
            Kind::Str => {
                let token = self.string()?;
                Ok(Value::Str { value: token.data })
            }
            Kind::Label | Kind::Operator | Kind::NamespacePointer => {
                // Rewind point: a bare `LABEL` is a label value, anything that
                // continues into `(` is an inline node.
                let start = self.pos;
                let (library, type_name) = self.type_name_namespace()?;

                if self.at(Kind::LParen) {
                    let attributes = self.connection_block()?;
                    return Ok(Value::NodeRef {
                        node: Box::new(Node::new(
                            type_name,
                            library,
                            Token::unspecified(),
                            attributes,
                        )),
                    });
                }

                if type_name_is_qualified(&library) {
                    return self.error("expected '(' after a qualified type name");
                }

                self.pos = start;
                let token = self.bump();
                Ok(Value::Label { token: ir_token(&token) })
            }
            Kind::Unrecognized => self.error("unrecognized token"),
            _ => self.error("expected a value"),
        }
    }
}

/// True when the type name carried an explicit namespace (`ns::type` or
/// `::type`), which can only be used for a node.
fn type_name_is_qualified(library: &Token) -> bool {
    library.specified
}
