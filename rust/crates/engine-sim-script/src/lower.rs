//! An identity-preserving arena for the IR.
//!
//! The parser produces a tree, and a tree is the wrong shape for everything
//! that comes after it: piranha resolves *in place* (`IrNode::m_definition`,
//! `IrAttributeDefinition::m_typeDefinition`), compares structures by pointer
//! (circular-reference detection), memoises per structure (`m_definitionsResolved`,
//! `m_expanded`, ...) and walks a `registerComponent` order that is not the
//! parse order. Every one of those needs stable identity and shared access.
//!
//! So the parse tree is *lowered* into a flat arena keyed by indices, with the
//! same fields and the same component edges `IrParserStructure` has. The parse
//! tree itself is untouched: the oracle bridge (`dump.rs`) keeps using it, and
//! [`Arena::to_unit`] round-trips back, which is what the tests check.
//!
//! Component order is load-bearing and is reproduced exactly:
//!
//! | structure | components (`registerComponent`) |
//! |---|---|
//! | `IrCompilationUnit` | every `addNode` / `addImportStatement` / `addNodeDefinition`, in call order |
//! | `IrNode` | its `IrAttributeList` |
//! | `IrNodeDefinition` | the `IrAttributeDefinitionList`, then the body `IrNodeList` |
//! | `IrAttributeDefinition` | the default value, when there is one |
//! | `IrAttribute` | its value |
//! | `IrAttribute(Definition)List`, `IrNodeList` | their items |
//! | `IrValueNodeRef` | the node it refers to |
//! | `IrBinaryOperator` | left, then right |
//! | `IrUnaryOperator` | the operand |

use crate::error::CompilationError;
use crate::ir::*;

pub type StructureId = usize;

/// `IrParserStructure`'s per-structure stage flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub definitions_resolved: bool,
    pub expanded: bool,
    pub validated: bool,
    pub types_checked: bool,
    pub instantiation_checked: bool,
    pub references_checked: bool,
}

/// A literal value, mirroring `IrValueConstant<T, TypeCode>`.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(i32),
    Float(f64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitData {
    pub path: String,
    /// Every `add*` call in source order: this is `m_components`.
    pub statements: Vec<StructureId>,
    pub imports: Vec<StructureId>,
    pub definitions: Vec<StructureId>,
    pub nodes: Vec<StructureId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeData {
    pub type_name: Token,
    pub library: Token,
    pub name: Token,
    pub attributes: StructureId,
    /// Resolved by `IrNode::resolveNodeDefinition`.
    pub definition: Option<StructureId>,
    pub is_interface: bool,
    pub build: bool,
    /// Set on the instance `auto node ...` creates.
    pub auto_instance: bool,
    /// The value an expanded literal carries. `IrLiteralNode<T>` injects it
    /// into the node it generates (`setData`); the node's *name* is only the
    /// `%g` spelling of it, so the exact value has to travel separately.
    pub literal: Option<Literal>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeDefinitionData {
    pub name: Token,
    pub is_inline: bool,
    pub defines_builtin: bool,
    pub builtin_name: Token,
    pub scope_token: Option<Token>,
    pub attributes: StructureId,
    pub body: Option<StructureId>,
    /// `auto node ...`: the definition owns an instance in the same unit.
    pub is_auto: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributeDefinitionData {
    pub direction: Direction,
    pub direction_token: Token,
    pub name: Token,
    pub is_alias: bool,
    pub alias_token: Token,
    pub type_info: Option<(Token, Token)>,
    pub default_value: Option<StructureId>,
    /// Resolved by `IrAttributeDefinition::_resolveDefinitions`.
    pub type_definition: Option<StructureId>,
    /// Back-reference set by the owning list during resolution.
    pub owner: Option<StructureId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributeData {
    pub name: Token,
    pub value: StructureId,
    /// Resolved by `IrNode::resolveAttributeDefinitions`.
    pub definition: Option<StructureId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportData {
    pub lib_name: Token,
    pub short_name: Option<Token>,
    /// Set by `Compiler::analyze`.
    pub unit: Option<StructureId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Unit(UnitData),
    Node(NodeData),
    NodeDefinition(NodeDefinitionData),
    AttributeDefinition(AttributeDefinitionData),
    Attribute(AttributeData),
    AttributeList(Vec<StructureId>),
    AttributeDefinitionList(Vec<StructureId>),
    NodeList(Vec<StructureId>),
    Import(ImportData),
    Literal { token: Token, literal: Literal },
    Label(Token),
    NodeRef(StructureId),
    Binary { token: Token, op: BinOp, left: StructureId, right: StructureId },
    /// `IrInternalReference`: a value an expansion created to point at an
    /// existing structure *in a fixed context*.
    InternalReference { structure: StructureId, context: Option<crate::context::ContextId> },
    Unary { token: Token, op: UnOp, operand: StructureId },
}

impl Kind {
    pub fn is_unit(&self) -> bool {
        matches!(self, Kind::Unit(_))
    }
}

impl NodeData {
    pub fn new(
        type_name: Token,
        library: Token,
        name: Token,
        attributes: StructureId,
    ) -> Self {
        NodeData {
            type_name,
            library,
            name,
            attributes,
            definition: None,
            is_interface: false,
            build: true,
            auto_instance: false,
            literal: None,
        }
    }
}

/// One `IrParserStructure`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The structure that registered this one (`registerComponent`).
    pub parent: Option<StructureId>,
    /// `m_scopeParent`: where name lookup continues from.
    pub scope_parent: Option<StructureId>,
    /// `m_logicalParent`: the structure this one is a member of.
    pub logical_parent: Option<StructureId>,
    /// `m_parentUnit`, resolved once lowering knows the enclosing unit.
    pub parent_unit: Option<StructureId>,
    /// `m_summaryToken`: the span of everything registered below this node.
    pub summary: Token,
    pub visibility: Visibility,
    pub default_visibility: Visibility,
    pub check_references: bool,
    pub flags: Flags,
    pub kind: Kind,
}

impl Entry {
    /// A structure that did not come from source: an expansion. It has no
    /// tokens, so its summary stays the "no position" span, and it is linked to
    /// its parent the way `setLogicalParent`/`setScopeParent` do.
    pub(crate) fn synthetic(parent: StructureId, parent_unit: Option<StructureId>) -> Self {
        Entry {
            parent: Some(parent),
            scope_parent: Some(parent),
            logical_parent: Some(parent),
            parent_unit,
            summary: Token::unspecified(),
            visibility: Visibility::Default,
            default_visibility: Visibility::Private,
            check_references: true,
            flags: Flags::default(),
            kind: Kind::AttributeList(Vec::new()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Arena {
    pub(crate) entries: Vec<Entry>,
    pub(crate) units: Vec<StructureId>,
    /// `IrCompilationUnit::addCompilationError` -- resolution and validation
    /// report through the unit, collected here in discovery order.
    pub(crate) errors: Vec<CompilationError>,
    /// Which `LanguageRules` to consult. Defaults to the engine's own, which is
    /// what every existing caller wants.
    pub(crate) rules: crate::builtin_types::Rules,
    /// `m_expansions` and `m_infiniteLoops`. The original keeps both on each
    /// structure, so they outlive one phase: expansion writes them and
    /// `checkReferences` reads them back. They therefore live here rather than
    /// in a `ReferenceState` local to a single walk.
    pub(crate) expansion_state: crate::reference::ReferenceState,
    /// The structures `build_graph` is currently generating, with their
    /// contexts. The original has no such stack -- it recurses until its stack
    /// is gone -- and one reachable script shape gets there: a builtin
    /// instance whose alias output has no default value (the "positive"
    /// operator's `NoOpNode`) has no port for `generateOutput` to find, so
    /// generation walks back to the same node forever.
    pub(crate) build_stack: Vec<(StructureId, crate::context::ContextId)>,
    /// The nodes the alias walk is standing on, for the same reason:
    /// `generateAliasNode` and `generateAliasOutput` follow interfaces back into
    /// nodes they already visited.
    pub(crate) alias_stack: Vec<(usize, bool)>,
}

impl Arena {
    /// Lowers every unit, in the order given, into one arena.
    pub fn lower(units: &[Unit]) -> Arena {
        let mut arena = Arena::default();
        for unit in units {
            arena.lower_unit(unit);
        }
        arena
    }

    /// Lowers a single unit.
    pub fn lower_unit(&mut self, unit: &Unit) -> StructureId {
        // The unit exists -- and knows it is a unit -- before anything below it
        // is lowered, because every child resolves its `parent_unit` by walking
        // up to it.
        let unit_id = self.begin(
            None,
            Kind::Unit(UnitData {
                path: unit.path.clone(),
                statements: Vec::new(),
                imports: Vec::new(),
                definitions: Vec::new(),
                nodes: Vec::new(),
            }),
        );

        let mut statements = Vec::new();
        let mut imports = Vec::new();
        let mut definitions = Vec::new();
        let mut nodes = Vec::new();

        for statement in &unit.statements {
            match statement {
                Statement::Import(import) => {
                    let data = ImportData {
                        lib_name: import.lib_name.clone(),
                        short_name: import.short_name.clone(),
                        unit: None,
                    };

                    let id = self.add_child(unit_id, Kind::Import(data));
                    self.entries[id].visibility = import.visibility;

                    imports.push(id);
                    statements.push(id);
                }
                Statement::Definition(definition) => {
                    let id = self.lower_definition(unit_id, definition, false);
                    definitions.push(id);
                    statements.push(id);
                }
                Statement::AutoDefinition(definition) => {
                    // `immediate_node_definition: AUTO specific_node_definition`
                    // adds the instance before it registers the definition, and
                    // the instance carries the definition's *name* -- which is
                    // what makes the definition reachable as a name from inside
                    // its own body.
                    let node_id = self.begin(
                        Some(unit_id),
                        Kind::Node(NodeData {
                            type_name: definition.name.clone(),
                            library: Token::unspecified(),
                            name: definition.name.clone(),
                            attributes: usize::MAX,
                            definition: None,
                            is_interface: false,
                            build: true,
                            auto_instance: true,
                            literal: None,
                        }),
                    );

                    let definition_id = self.lower_definition(unit_id, definition, true);

                    let attributes =
                        self.lower_attributes(node_id, &[]);
                    let Kind::Node(data) = &mut self.entries[node_id].kind else {
                        unreachable!()
                    };
                    data.attributes = attributes;
                    data.definition = Some(definition_id);
                    self.finish(node_id);

                    nodes.push(node_id);
                    definitions.push(definition_id);
                    statements.push(node_id);
                    statements.push(definition_id);
                }
                Statement::Node(node) => {
                    let id = self.lower_node(unit_id, node);
                    nodes.push(id);
                    statements.push(id);
                }
            }
        }

        let Kind::Unit(data) = &mut self.entries[unit_id].kind else {
            unreachable!()
        };
        data.statements = statements;
        data.imports = imports;
        data.definitions = definitions;
        data.nodes = nodes;

        self.finish(unit_id);
        self.units.push(unit_id);

        unit_id
    }

    /// Selects the rule set (`IrCompilationUnit::setRules`).
    pub fn set_rules(&mut self, rules: crate::builtin_types::Rules) {
        self.rules = rules;
    }

    pub fn rules(&self) -> crate::builtin_types::Rules {
        self.rules
    }

    pub fn errors(&self) -> &[CompilationError] {
        &self.errors
    }

    pub fn take_errors(&mut self) -> Vec<CompilationError> {
        std::mem::take(&mut self.errors)
    }

    /// Links every import statement to the unit it resolved to
    /// (`IrImportStatement::setUnit`, set by `Compiler::analyze`).
    pub fn link_imports(&mut self, import_paths: &[Vec<String>]) {
        let mut by_path = std::collections::BTreeMap::new();
        for unit in &self.units {
            if let Kind::Unit(data) = &self.entries[*unit].kind {
                by_path.insert(data.path.clone(), *unit);
            }
        }

        for (unit, paths) in self.units.clone().iter().zip(import_paths) {
            let imports = match &self.entries[*unit].kind {
                Kind::Unit(data) => data.imports.clone(),
                _ => continue,
            };

            for (import, path) in imports.iter().zip(paths) {
                let target = by_path.get(path).copied();
                if let Kind::Import(data) = &mut self.entries[*import].kind {
                    data.unit = target;
                }
            }
        }
    }

    pub fn units(&self) -> &[StructureId] {
        &self.units
    }

    pub fn entry(&self, id: StructureId) -> &Entry {
        &self.entries[id]
    }

    pub fn entry_mut(&mut self, id: StructureId) -> &mut Entry {
        &mut self.entries[id]
    }

    pub fn kind(&self, id: StructureId) -> &Kind {
        &self.entries[id].kind
    }

    /// `IrParserStructure::getComponentCount` / `getComponent(i)`.
    pub fn components(&self, id: StructureId) -> Vec<StructureId> {
        match &self.entries[id].kind {
            Kind::Unit(data) => data.statements.clone(),
            Kind::Node(data) => vec![data.attributes],
            Kind::NodeDefinition(data) => match data.body {
                // `setAttributeDefinitionList` then `setBody`.
                Some(body) => vec![data.attributes, body],
                None => vec![data.attributes],
            },
            Kind::AttributeDefinition(data) => match data.default_value {
                Some(value) => vec![value],
                None => Vec::new(),
            },
            Kind::Attribute(data) => vec![data.value],
            Kind::AttributeList(items) => items.clone(),
            Kind::AttributeDefinitionList(items) => items.clone(),
            Kind::NodeList(items) => items.clone(),
            Kind::NodeRef(node) => vec![*node],
            Kind::InternalReference { .. } => Vec::new(),
            Kind::Binary { left, right, .. } => vec![*left, *right],
            Kind::Unary { operand, .. } => vec![*operand],
            Kind::Import(_) | Kind::Literal { .. } | Kind::Label(_) => Vec::new(),
        }
    }

    /// Walks a structure and everything below it, pre-order, the way
    /// `IrParserStructure::resolveDefinitions` and friends do.
    pub fn visit(&self, root: StructureId, mut visit: impl FnMut(StructureId)) {
        self.visit_impl(root, &mut visit);
    }

    fn visit_impl(&self, root: StructureId, visit: &mut dyn FnMut(StructureId)) {
        visit(root);
        for child in self.components(root) {
            self.visit_impl(child, visit);
        }
    }

    /// Rebuilds the parse-tree form of a unit. Used by the tests to prove the
    /// lowering is lossless.
    pub fn to_unit(&self, unit_id: StructureId) -> Unit {
        let Kind::Unit(data) = &self.entries[unit_id].kind else {
            panic!("not a unit");
        };

        let mut statements = Vec::new();
        for statement in &data.statements {
            match self.kind(*statement) {
                Kind::Import(_) => statements.push(Statement::Import(self.to_import(*statement))),
                Kind::NodeDefinition(definition) => {
                    let parsed = self.to_definition(*statement);
                    statements.push(if definition.is_auto {
                        Statement::AutoDefinition(parsed)
                    }
                    else {
                        Statement::Definition(parsed)
                    });
                }
                Kind::Node(node) if node.auto_instance => {
                    // The instance is implied by `Statement::AutoDefinition`.
                }
                Kind::Node(_) => statements.push(Statement::Node(self.to_node(*statement))),
                other => panic!("unexpected unit component {:?}", other),
            }
        }

        Unit { path: data.path.clone(), statements }
    }

    // ------------------------------------------------------------- lowering

    fn reserve(&mut self) -> StructureId {
        self.entries.push(Entry {
            parent: None,
            scope_parent: None,
            logical_parent: None,
            parent_unit: None,
            summary: Token::unspecified(),
            visibility: Visibility::Default,
            default_visibility: Visibility::Private,
            check_references: true,
            flags: Flags::default(),
            kind: Kind::AttributeList(Vec::new()),
        });

        self.entries.len() - 1
    }

    /// Reserves a structure and links it to `parent` *before* its children are
    /// lowered, so they can already see the scope chain above them.
    fn begin(&mut self, parent: Option<StructureId>, kind: Kind) -> StructureId {
        let id = self.reserve();
        let parent_unit = parent.and_then(|parent| self.unit_of(parent));

        let entry = &mut self.entries[id];
        entry.parent = parent;
        entry.scope_parent = parent;
        entry.logical_parent = parent;
        entry.parent_unit = parent_unit;
        entry.kind = kind;

        id
    }

    /// `registerComponent` receives the finished child: its span is folded into
    /// the parent's summary.
    fn finish(&mut self, id: StructureId) {
        self.compute_summary(id);
    }

    fn add_child(&mut self, parent: StructureId, kind: Kind) -> StructureId {
        let child = self.begin(Some(parent), kind);
        self.finish(child);
        child
    }

    /// `IrValueConstant`'s constructor calls `useToken`, which registers the
    /// literal's token, so the literal's summary *is* that token and every
    /// enclosing summary folds it in. An attribute's summary therefore spans
    /// its value, which is what the original reports positions from.
    fn add_literal(&mut self, parent: StructureId, token: Token, literal: Literal) -> StructureId {
        let child = self.add_child(parent, Kind::Literal { token: token.clone(), literal });
        self.entries[child].summary = token;
        child
    }

    /// Push a structure built after lowering (an expansion). It has no tokens,
    /// so its summary stays the "no position" span, and it is linked to its
    /// parent the way `setLogicalParent`/`setScopeParent` do.
    pub(crate) fn push_entry(&mut self, entry: Entry) -> StructureId {
        self.entries.push(entry);
        self.entries.len() - 1
    }

    /// The enclosing unit of a structure, if it has one.
    pub(crate) fn unit_of(&self, id: StructureId) -> Option<StructureId> {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if self.entries[candidate].kind.is_unit() {
                return Some(candidate);
            }
            current = self.entries[candidate].parent;
        }

        None
    }

    /// `registerToken` + `registerComponent` both feed `m_summaryToken`, which
    /// is a min/max merge of the spans -- so the order they were registered in
    /// does not matter.
    fn compute_summary(&mut self, id: StructureId) {
        let mut summary = Token::unspecified();

        for token in self.own_tokens(id) {
            summary = combine(summary, &token);
        }

        for child in self.components(id) {
            summary = combine(summary, &self.entries[child].summary.clone());
        }

        self.entries[id].summary = summary;
    }

    fn own_tokens(&self, id: StructureId) -> Vec<Token> {
        match &self.entries[id].kind {
            Kind::Node(data) => vec![data.type_name.clone(), data.name.clone(), data.library.clone()],
            Kind::NodeDefinition(data) => {
                let mut tokens = vec![data.name.clone()];
                if data.defines_builtin {
                    tokens.push(data.builtin_name.clone());
                }
                if let Some(scope) = &data.scope_token {
                    tokens.push(scope.clone());
                }
                tokens
            }
            Kind::AttributeDefinition(data) => {
                let mut tokens = vec![data.name.clone(), data.direction_token.clone()];
                if data.is_alias {
                    tokens.push(data.alias_token.clone());
                }
                if let Some((library, type_name)) = &data.type_info {
                    tokens.push(library.clone());
                    tokens.push(type_name.clone());
                }
                tokens
            }
            Kind::Attribute(data) => vec![data.name.clone()],
            Kind::Import(data) => vec![data.lib_name.clone()],
            Kind::Label(token) => vec![token.clone()],
            _ => Vec::new(),
        }
    }

    fn lower_definition(
        &mut self,
        parent: StructureId,
        definition: &NodeDefinition,
        is_auto: bool,
    ) -> StructureId {
        let id = self.begin(
            Some(parent),
            Kind::NodeDefinition(NodeDefinitionData {
                name: definition.name.clone(),
                is_inline: definition.is_inline,
                defines_builtin: definition.defines_builtin,
                builtin_name: definition.builtin_name.clone(),
                scope_token: definition.scope_token.clone(),
                attributes: usize::MAX,
                body: None,
                is_auto,
            }),
        );

        // `IrNodeDefinition`'s constructor makes definitions public by default.
        self.entries[id].visibility = definition.visibility;
        self.entries[id].default_visibility = Visibility::Public;

        // `setAttributeDefinitionList` then `setBody`.
        let attributes = self.begin(Some(id), Kind::AttributeDefinitionList(Vec::new()));
        let mut ports = Vec::new();
        for port in &definition.ports {
            ports.push(self.lower_port(attributes, port));
        }
        let Kind::AttributeDefinitionList(items) = &mut self.entries[attributes].kind else {
            unreachable!()
        };
        *items = ports;
        self.finish(attributes);

        let body = definition.body.as_ref().map(|nodes| {
            let body_id = self.begin(Some(id), Kind::NodeList(Vec::new()));
            let mut items = Vec::new();
            for node in nodes {
                items.push(self.lower_node(body_id, node));
            }
            let Kind::NodeList(list) = &mut self.entries[body_id].kind else {
                unreachable!()
            };
            *list = items;
            self.finish(body_id);
            body_id
        });

        let Kind::NodeDefinition(data) = &mut self.entries[id].kind else {
            unreachable!()
        };
        data.attributes = attributes;
        data.body = body;

        self.finish(id);

        id
    }

    fn lower_port(&mut self, parent: StructureId, port: &AttributeDefinition) -> StructureId {
        let id = self.begin(
            Some(parent),
            Kind::AttributeDefinition(AttributeDefinitionData {
                direction: port.direction,
                direction_token: port.direction_token.clone(),
                name: port.name.clone(),
                is_alias: port.is_alias,
                alias_token: port.alias_token.clone(),
                type_info: port.type_info.clone(),
                default_value: None,
                type_definition: None,
                owner: None,
            }),
        );

        // Both `IrAttributeDefinition` constructors call
        // `setVisibility(IrVisibility::Public)`, so `allowsExternalAccess()` is
        // true for every port -- `IrParserStructure`'s own default is private.
        self.entries[id].visibility = Visibility::Public;

        let default_value = port.default_value.as_ref().map(|value| self.lower_value(id, value));

        let Kind::AttributeDefinition(data) = &mut self.entries[id].kind else {
            unreachable!()
        };
        data.default_value = default_value;

        self.finish(id);

        id
    }

    fn lower_attributes(&mut self, node: StructureId, attributes: &[Attribute]) -> StructureId {
        let list = self.begin(Some(node), Kind::AttributeList(Vec::new()));

        let mut ids = Vec::new();
        for attribute in attributes {
            let id = self.begin(
                Some(list),
                Kind::Attribute(AttributeData {
                    name: attribute.name.clone(),
                    value: usize::MAX,
                    definition: None,
                }),
            );

            // Every `IrAttribute` constructor calls
            // `setVisibility(IrVisibility::Public)`, so an instance's attributes
            // are externally accessible (`myNode.A`) even though
            // `IrParserStructure`'s own default is private. Body *nodes* keep
            // that private default, which is what `R0061` reports.
            self.entries[id].visibility = Visibility::Public;

            let value = self.lower_value(id, &attribute.value);
            let Kind::Attribute(data) = &mut self.entries[id].kind else {
                unreachable!()
            };
            data.value = value;
            self.finish(id);

            ids.push(id);
        }

        let Kind::AttributeList(items) = &mut self.entries[list].kind else {
            unreachable!()
        };
        *items = ids;

        // `IrNode::setScopeParent` forwards the *node's* parent scope to the
        // attribute list, and it is called whenever the node is registered as a
        // component -- so an attribute's value looks names up in the node's
        // enclosing scope, never in the node itself. Without this a label that
        // happens to equal an attribute name (`i(x: 5, y: x)`) resolves to the
        // attribute and reports a circular reference where the original reports
        // an unresolved one.
        self.entries[list].scope_parent = self.entries[node].scope_parent;

        self.finish(list);

        list
    }

    fn lower_node(&mut self, parent: StructureId, node: &Node) -> StructureId {
        let id = self.begin(
            Some(parent),
            Kind::Node(NodeData {
                type_name: node.type_name.clone(),
                library: node.library.clone(),
                name: node.name.clone(),
                attributes: usize::MAX,
                definition: None,
                is_interface: false,
                build: true,
                auto_instance: false,
                literal: None,
            }),
        );

        let attributes = self.lower_attributes(id, &node.attributes);

        let Kind::Node(data) = &mut self.entries[id].kind else {
            unreachable!()
        };
        data.attributes = attributes;

        self.finish(id);

        id
    }

    fn lower_value(&mut self, parent: StructureId, value: &Value) -> StructureId {
        match value {
            Value::Int { token, value } => {
                self.add_literal(parent, token.clone(), Literal::Int(*value))
            }
            Value::Float { token, value } => {
                self.add_literal(parent, token.clone(), Literal::Float(*value))
            }
            Value::Bool { token, value } => {
                self.add_literal(parent, token.clone(), Literal::Bool(*value))
            }
            Value::Str { token, value } => {
                self.add_literal(parent, token.clone(), Literal::Str(value.clone()))
            }
            Value::Label { token } => self.add_child(parent, Kind::Label(token.clone())),
            Value::NodeRef { node } => {
                let id = self.begin(Some(parent), Kind::NodeRef(usize::MAX));
                let node_id = self.lower_node(id, node);
                let Kind::NodeRef(inner) = &mut self.entries[id].kind else {
                    unreachable!()
                };
                *inner = node_id;
                self.finish(id);
                id
            }
            Value::Binary { token, op, left, right } => {
                let id = self.begin(Some(parent), Kind::Binary {
                    token: token.clone(), op: *op, left: usize::MAX, right: usize::MAX,
                });
                let left = self.lower_value(id, left);
                let right = self.lower_value(id, right);

                // `IrBinaryOperator`'s constructor: the right-hand operand of a
                // `.` "will *always* be a standard label that doesn't reference
                // anything itself", so it stops being checked -- neither
                // expanded nor reported as an unresolved reference.
                if *op == BinOp::Dot {
                    self.entries[right].check_references = false;
                }

                let Kind::Binary { left: left_slot, right: right_slot, .. } =
                    &mut self.entries[id].kind
                else {
                    unreachable!()
                };
                *left_slot = left;
                *right_slot = right;
                self.finish(id);
                id
            }
            Value::Unary { token, op, operand } => {
                let id = self.begin(Some(parent), Kind::Unary {
                    token: token.clone(), op: *op, operand: usize::MAX,
                });
                let operand = self.lower_value(id, operand);
                let Kind::Unary { operand: slot, .. } = &mut self.entries[id].kind else {
                    unreachable!()
                };
                *slot = operand;
                self.finish(id);
                id
            }
        }
    }

    fn to_import(&self, id: StructureId) -> ImportStatement {
        let Kind::Import(data) = self.kind(id) else {
            panic!("not an import");
        };

        ImportStatement {
            visibility: self.entries[id].visibility,
            lib_name: data.lib_name.clone(),
            short_name: data.short_name.clone(),
        }
    }

    fn to_definition(&self, id: StructureId) -> NodeDefinition {
        let Kind::NodeDefinition(data) = self.kind(id) else {
            panic!("not a definition");
        };

        let Kind::AttributeDefinitionList(ports) = self.kind(data.attributes) else {
            panic!("definition without a port list");
        };

        let ports = ports.iter().map(|port| self.to_port(*port)).collect();

        let body = data.body.map(|body| {
            let Kind::NodeList(items) = self.kind(body) else {
                panic!("definition body is not a node list");
            };
            items.iter().map(|node| self.to_node(*node)).collect()
        });

        NodeDefinition {
            name: data.name.clone(),
            is_inline: data.is_inline,
            defines_builtin: data.defines_builtin,
            builtin_name: data.builtin_name.clone(),
            visibility: self.entries[id].visibility,
            scope_token: data.scope_token.clone(),
            ports,
            body,
        }
    }

    fn to_port(&self, id: StructureId) -> AttributeDefinition {
        let Kind::AttributeDefinition(data) = self.kind(id) else {
            panic!("not a port");
        };

        AttributeDefinition {
            direction: data.direction,
            direction_token: data.direction_token.clone(),
            name: data.name.clone(),
            is_alias: data.is_alias,
            alias_token: data.alias_token.clone(),
            type_info: data.type_info.clone(),
            default_value: data.default_value.map(|value| self.to_value(value)),
        }
    }

    fn to_node(&self, id: StructureId) -> Node {
        let Kind::Node(data) = self.kind(id) else {
            panic!("not a node");
        };

        let Kind::AttributeList(attributes) = self.kind(data.attributes) else {
            panic!("node without an attribute list");
        };

        Node {
            type_name: data.type_name.clone(),
            library: data.library.clone(),
            name: data.name.clone(),
            attributes: attributes.iter().map(|attribute| self.to_attribute(*attribute)).collect(),
        }
    }

    fn to_attribute(&self, id: StructureId) -> Attribute {
        let Kind::Attribute(data) = self.kind(id) else {
            panic!("not an attribute");
        };

        Attribute { name: data.name.clone(), value: self.to_value(data.value) }
    }

    fn to_value(&self, id: StructureId) -> Value {
        match self.kind(id) {
            Kind::Literal { token, literal } => match literal {
                Literal::Int(value) => Value::Int { token: token.clone(), value: *value },
                Literal::Float(value) => Value::Float { token: token.clone(), value: *value },
                Literal::Bool(value) => Value::Bool { token: token.clone(), value: *value },
                Literal::Str(value) => Value::Str { token: token.clone(), value: value.clone() },
            },
            Kind::Label(token) => Value::Label { token: token.clone() },
            Kind::NodeRef(node) => Value::NodeRef { node: Box::new(self.to_node(*node)) },
            Kind::Binary { token, op, left, right } => Value::Binary {
                token: token.clone(),
                op: *op,
                left: Box::new(self.to_value(*left)),
                right: Box::new(self.to_value(*right)),
            },
            Kind::Unary { token, op, operand } => {
                Value::Unary {
                    token: token.clone(), op: *op,
                    operand: Box::new(self.to_value(*operand)),
                }
            }
            other => panic!("not a value: {:?}", other),
        }
    }
}

/// `IrTokenInfo::combine`
fn combine(left: Token, right: &Token) -> Token {
    let mut combined = left;

    if right.line == 0 {
        return combined;
    }

    // A default `IrTokenInfo` has an "infinite" start, so any real token wins;
    // here line 0 is that sentinel.
    if combined.line == 0 || right.line < combined.line {
        combined.line = right.line;
        combined.col = right.col;
    }
    else if right.line == combined.line && right.col < combined.col {
        combined.col = right.col;
    }

    combined
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_unit;

    fn lower_one(source: &str) -> (Arena, StructureId) {
        let unit = parse_unit("test.mr", source).expect("parses");
        let mut arena = Arena::default();
        let id = arena.lower_unit(&unit);
        (arena, id)
    }

    #[test]
    fn lowering_round_trips_a_definition() {
        let source = "public node foo => __builtin {\n  input a: 1 + 2;\n  alias output b [ns::type];\n}\n";
        let (arena, unit_id) = lower_one(source);

        let original = parse_unit("test.mr", source).unwrap();
        assert_eq!(arena.to_unit(unit_id), original);
    }

    #[test]
    fn lowering_round_trips_nodes_and_bodies() {
        let source = "node a {\n  input x;\n  b(1)\n  c.d(2)\n}\nwires w()\nf(1).g(2)\n";
        let (arena, unit_id) = lower_one(source);

        let original = parse_unit("test.mr", source).unwrap();
        assert_eq!(arena.to_unit(unit_id), original);
    }

    #[test]
    fn components_are_in_register_component_order() {
        let (arena, unit_id) = lower_one("node a {\n  input x;\n  b(1)\n}\n");

        // Unit -> [definition]; definition -> [port list, body].
        let unit_components = arena.components(unit_id);
        assert_eq!(unit_components.len(), 1);

        let definition = unit_components[0];
        let components = arena.components(definition);
        assert_eq!(components.len(), 2);

        assert!(matches!(arena.kind(components[0]), Kind::AttributeDefinitionList(_)));
        assert!(matches!(arena.kind(components[1]), Kind::NodeList(_)));
    }

    #[test]
    fn summary_tokens_span_the_subtree() {
        let source = "node a {\n  input x;\n}\n";
        let (arena, unit_id) = lower_one(source);

        // The unit and the definition both start on line 1; the port list ends
        // on line 3.
        assert_eq!(arena.entry(unit_id).summary.line, 1);

        let definition = arena.components(unit_id)[0];
        assert_eq!(arena.entry(definition).summary.line, 1);
    }

    #[test]
    fn auto_definitions_lower_to_a_node_and_a_definition() {
        let (arena, unit_id) = lower_one("auto node b { input x; }\n");

        let Kind::Unit(data) = arena.kind(unit_id) else {
            panic!("not a unit");
        };

        assert_eq!(data.statements.len(), 2);
        assert_eq!(data.nodes.len(), 1);
        assert_eq!(data.definitions.len(), 1);

        let node = data.nodes[0];
        let definition = data.definitions[0];

        let Kind::Node(node_data) = arena.kind(node) else {
            panic!("not a node");
        };
        assert!(node_data.auto_instance);
        assert_eq!(node_data.definition, Some(definition));
    }

    #[test]
    fn parent_unit_is_filled_in() {
        let (arena, unit_id) = lower_one("node a {\n  input x;\n}\n");

        let definition = arena.components(unit_id)[0];
        assert_eq!(arena.entry(definition).parent_unit, Some(unit_id));

        let ports = arena.components(definition)[0];
        assert_eq!(arena.entry(ports).parent_unit, Some(unit_id));

        let port = arena.components(ports)[0];
        assert_eq!(arena.entry(port).parent_unit, Some(unit_id));
    }

    #[test]
    fn visit_walks_the_whole_subtree() {
        let (arena, unit_id) = lower_one("node a {\n  input x: f(1);\n}\n");

        let mut count = 0;
        arena.visit(unit_id, |_| count += 1);

        // unit, definition, port list, port, node ref, referenced node,
        // attribute list, attribute, literal.
        assert_eq!(count, 9);
    }
}
