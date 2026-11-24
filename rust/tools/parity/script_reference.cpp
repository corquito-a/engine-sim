// Reference oracle for the `.mr` front end.
//
// The original front end is a flex scanner plus a bison LALR parser, and
// neither tool exists in this environment. Everything downstream of the parser
// does compile, though: the piranha IR, definition resolution, validation, node
// generation and the whole `scripting/` binding layer. This driver feeds the
// IR that `engine-sim-script` (the Rust port) produced straight into those
// stages, which is what makes the port verifiable:
//
//   script_reference --ir  <dump.txt>
//       Rebuild the piranha IR from the Rust dump and re-emit it. Comparing
//       the two texts proves the Rust parser produced the same IR objects the
//       original parser would have (see `compare_ir.py`).
//
//   script_reference --run <root.mr> <dump.txt> [steps]
//   script_reference --resolved <root.mr> <dump.txt>
//   script_reference --errors <root.mr> <dump.txt>
//       Rebuild the IR, run resolve/validate/build/execute exactly like
//       `EngineSimApplication::loadScript`, then dump the resulting engine and
//       a deterministic simulation trace.
//
// The dump format is produced by `engine_sim_script::dump`; see that module for
// the grammar. Only `specified` is carried for tokens (`empty` and `valid` are
// never consulted by piranha or the scripting layer).

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <vector>

// The re-emitter needs to read a handful of tokens that piranha keeps
// protected and exposes only through lossy accessors (`IrNode::getTypeToken()`
// slices away `data`). Widening access for one translation unit keeps the
// reference build byte-identical to the original sources.
#define protected public
#include <piranha/include/piranha.h>
#undef protected

#include "core_trace_common.h"

// The shell of the scripting layer, minus the window/rendering half.
// `piranha/test/test_rules.cpp`: the rules the original's own test suite runs
// against. Selected with `PARITY_RULES=test`; see `run_conformance.py`.
#include "test_rules.h"

#include "compiler.h"
#include "actions.h"
#include "vehicle.h"
#include "transmission.h"

// ------------------------------------------------------------ S-expressions

namespace sx {

struct Node {
    bool isList = false;
    std::string atom;
    std::vector<Node> items;

    /// First child whose head atom is `head`.
    const Node *child(const std::string &head) const {
        for (const Node &item : items) {
            if (!item.isList || item.items.empty() || item.items[0].isList) continue;
            if (item.items[0].atom == head) return &item;
        }

        return nullptr;
    }

    const std::string &head() const { return items[0].atom; }

    bool flag(int index) const { return items[index].atom == "1"; }

    std::string text(int index) const { return items[index].atom; }

    int count() const { return (int)items.size(); }
};

class Reader {
public:
    explicit Reader(const std::string &text) : m_text(text) { /* void */ }

    bool eof() {
        skip();
        return m_pos >= m_text.size();
    }

    Node read() {
        skip();
        Node node;

        if (m_pos >= m_text.size()) {
            fail("unexpected end of input");
        }

        if (m_text[m_pos] == '(') {
            ++m_pos;
            node.isList = true;

            for (;;) {
                skip();
                if (m_pos >= m_text.size()) fail("unterminated list");
                if (m_text[m_pos] == ')') {
                    ++m_pos;
                    break;
                }
                node.items.push_back(read());
            }
        }
        else if (m_text[m_pos] == '"') {
            node.atom = readString();
        }
        else {
            const size_t start = m_pos;
            while (m_pos < m_text.size()) {
                const char c = m_text[m_pos];
                if (std::isspace((unsigned char)c) || c == '(' || c == ')' || c == '"') break;
                ++m_pos;
            }
            node.atom = m_text.substr(start, m_pos - start);
        }

        return node;
    }

private:
    void fail(const char *message) {
        std::fprintf(stderr, "ir dump: %s at offset %zu\n", message, m_pos);
        std::exit(1);
    }

    void skip() {
        while (m_pos < m_text.size()) {
            const char c = m_text[m_pos];
            if (std::isspace((unsigned char)c)) {
                ++m_pos;
            }
            else if (c == ';') {
                while (m_pos < m_text.size() && m_text[m_pos] != '\n') ++m_pos;
            }
            else {
                break;
            }
        }
    }

    std::string readString() {
        std::string out;
        ++m_pos; // opening quote

        while (m_pos < m_text.size()) {
            const char c = m_text[m_pos++];
            if (c == '"') return out;

            if (c != '\\') {
                out.push_back(c);
                continue;
            }

            if (m_pos >= m_text.size()) break;
            const char escaped = m_text[m_pos++];
            switch (escaped) {
                case 'n': out.push_back('\n'); break;
                case 't': out.push_back('\t'); break;
                case 'r': out.push_back('\r'); break;
                default: out.push_back(escaped); break;
            }
        }

        fail("unterminated string");
        return out;
    }

    const std::string &m_text;
    size_t m_pos = 0;
};

/// Emits a string the way Rust's `{:?}` + explicit escaping does.
static void writeString(std::string &out, const std::string &text) {
    out.push_back('"');
    for (char c : text) {
        switch (c) {
            case '"': out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n"; break;
            case '\t': out += "\\t"; break;
            case '\r': out += "\\r"; break;
            default: out.push_back(c); break;
        }
    }
    out.push_back('"');
}

} // namespace sx

// --------------------------------------------------------------- IR builder

using piranha::IrAttribute;
using piranha::IrAttributeDefinition;
using piranha::IrAttributeDefinitionList;
using piranha::IrAttributeList;
using piranha::IrBinaryOperator;
using piranha::IrCompilationUnit;
using piranha::IrImportStatement;
using piranha::IrNode;
using piranha::IrNodeDefinition;
using piranha::IrNodeList;
using piranha::IrTokenInfo_bool;
using piranha::IrTokenInfo_float;
using piranha::IrTokenInfo_int;
using piranha::IrTokenInfo_string;
using piranha::IrUnaryOperator;
using piranha::IrValue;
using piranha::IrValueBool;
using piranha::IrValueFloat;
using piranha::IrValueInt;
using piranha::IrValueLabel;
using piranha::IrValueNodeRef;
using piranha::IrValueString;
using piranha::IrVisibility;

/// Builds a token from either a bare `(tok "data" 0|1)` or a field wrapper that
/// contains one (`(name (tok "data" 0|1))`).
static IrTokenInfo_string buildToken(const sx::Node &form) {
    const sx::Node &wrapper = form.head() == "tok" ? form : form.items[1];

    IrTokenInfo_string token(wrapper.items[1].atom, 1, 1, 1, 1);
    token.specified = wrapper.flag(2);
    token.empty = !token.specified;

    // `(tok "text" specified line)` when the dump carries lines; the IR
    // round-trip dump does not, and then the default stands.
    if (wrapper.items.size() > 3 && token.specified) {
        // An unspecified token has the default `IrTokenInfo()` span
        // (`lineStart == INT_MAX`), which is what keeps it from lowering a
        // summary's line; only a specified token carries a source position.
        token.lineStart = token.lineEnd = std::atoi(wrapper.items[3].atom.c_str());
    }

    return token;
}

static IrValue *buildValue(const sx::Node &form);

static IrNode *buildNode(const sx::Node &form) {
    const sx::Node *type = form.child("type");
    const sx::Node *library = form.child("ns");
    const sx::Node *name = form.child("name");
    const sx::Node *attributes = form.child("attrs");

    auto *list = new IrAttributeList;

    if (attributes != nullptr) {
        for (const sx::Node &entry : attributes->items) {
            if (entry.isList && entry.head() == "attr") {
                const sx::Node &attributeName = entry.items[1];
                IrValue *value = buildValue(entry.items[2]);
                list->addAttribute(
                    attributeName.items[1].atom.empty()
                        ? new IrAttribute(value)
                        : new IrAttribute(buildToken(attributeName), value));
            }
        }
    }

    return new IrNode(buildToken(*type), buildToken(*name), list, buildToken(*library));
}

static IrValue *buildValue(const sx::Node &form) {
    const std::string &head = form.head();

    // `(int 42)` or, when the dump carries lines, `(int (tok "42" 1 15) 42)`.
    // The token matters: `IrValueConstant`'s constructor registers it, so the
    // literal's line is folded into every enclosing summary token, which is
    // where error locations come from.
    const bool constantsCarryTokens =
        form.items.size() > 2 && form.items[1].isList;
    const int constantBase = constantsCarryTokens ? 2 : 1;
    const int constantLine = constantsCarryTokens ? buildToken(form.items[1]).lineStart : 1;

    if (head == "int") {
        return new IrValueInt(
            IrTokenInfo_int(std::stoi(form.items[constantBase].atom),
                constantLine, constantLine, 1, 1));
    }

    if (head == "float") {
        return new IrValueFloat(
            IrTokenInfo_float(std::stod(form.items[constantBase].atom),
                constantLine, constantLine, 1, 1));
    }

    if (head == "bool") {
        return new IrValueBool(
            IrTokenInfo_bool(form.items[constantBase].atom == "1",
                constantLine, constantLine, 1, 1));
    }

    if (head == "str") {
        return new IrValueString(
            IrTokenInfo_string(form.items[constantBase].atom,
                constantLine, constantLine, 1, 1));
    }

    if (head == "label") {
        return new IrValueLabel(buildToken(form.items[1]));
    }

    if (head == "nref") {
        piranha::T_IrTokenInfo<IrNode *> info;
        info.data = buildNode(form.items[1]);
        return new IrValueNodeRef(info);
    }

    if (head == "bin") {
        const std::string &op = form.items[1].atom;
        // `(bin add (tok "+" 1 26) l r)` when the dump carries lines.
        const bool withToken = form.items[2].isList;
        const int valueBase = withToken ? 3 : 2;
        IrBinaryOperator::Operator operation = IrBinaryOperator::Operator::Dot;
        if (op == "add") operation = IrBinaryOperator::Operator::Add;
        else if (op == "sub") operation = IrBinaryOperator::Operator::Sub;
        else if (op == "mul") operation = IrBinaryOperator::Operator::Mul;
        else if (op == "div") operation = IrBinaryOperator::Operator::Div;

        IrBinaryOperator *binary = new IrBinaryOperator(
            operation, buildValue(form.items[valueBase]), buildValue(form.items[valueBase + 1]));

        if (withToken) {
            binary->m_summaryToken = buildToken(form.items[2]);
        }

        return binary;
    }

    if (head == "un") {
        const std::string &op = form.items[1].atom;
        const bool withToken = form.items[2].isList;
        const int valueBase = withToken ? 3 : 2;
        IrUnaryOperator::Operator operation = IrUnaryOperator::Operator::Default;
        if (op == "neg") operation = IrUnaryOperator::Operator::NumericNegate;
        else if (op == "not") operation = IrUnaryOperator::Operator::BoolNegate;
        else if (op == "pos") operation = IrUnaryOperator::Operator::Positive;

        IrUnaryOperator *unary =
            new IrUnaryOperator(operation, buildValue(form.items[valueBase]));

        if (withToken) {
            unary->m_summaryToken = buildToken(form.items[2]);
        }

        return unary;
    }

    std::fprintf(stderr, "ir dump: unknown value form '%s'\n", head.c_str());
    std::exit(1);
}

static IrAttributeDefinition *buildPort(const sx::Node &form) {
    const sx::Node *direction = form.child("dir");
    const sx::Node *name = form.child("name");
    const sx::Node *aliasToken = form.child("alias_tok");
    const sx::Node *type = form.child("type");
    const sx::Node *defaultValue = form.child("default");

    const std::string &dir = direction->items[1].atom;
    IrAttributeDefinition::Direction resolved = IrAttributeDefinition::Direction::Input;
    if (dir == "output") resolved = IrAttributeDefinition::Direction::Output;
    else if (dir == "modify") resolved = IrAttributeDefinition::Direction::Modify;
    else if (dir == "toggle") resolved = IrAttributeDefinition::Direction::Toggle;

    IrTokenInfo_string directionToken(dir, 1, 1, 1, 1);

    auto *definition =
        new IrAttributeDefinition(directionToken, buildToken(*name), resolved);

    if (form.child("alias")->flag(1)) {
        definition->setAlias(true);
        definition->setAliasToken(buildToken(*aliasToken));
    }
    else {
        definition->setAlias(false);
    }

    // `IrAttributeDefinition`'s constructor initialises `m_typeDefinition` and
    // nothing else, so `m_typeInfo` is uninitialised memory unless it is always
    // written. The grammar's action writes it for every port -- a default
    // (unspecified) token when the script wrote no `[type]` -- and a garbage
    // `specified` bit here makes `_resolveDefinitions` look up the empty type
    // name and report `UndefinedNodeType` for ports that have no type at all.
    piranha::IrTokenInfoSet<std::string, 2> info;
    info.data[0] = IrTokenInfo_string();
    info.data[1] = IrTokenInfo_string();

    if (type != nullptr) {
        info.data[0] = buildToken(*type->child("ns"));
        info.data[1] = buildToken(*type->child("nm"));
    }

    definition->setTypeInfo(info);

    // `IrAttributeDefinition` leaves `m_defaultValue` (and `m_isAlias`)
    // uninitialised: the grammar's actions are what set them, so the injection
    // has to set both whether or not the script wrote a value.
    definition->setDefaultValue(
        (defaultValue != nullptr) ? buildValue(defaultValue->items[1]) : nullptr);

    return definition;
}

static void buildDefinition(const sx::Node &form, IrCompilationUnit &unit) {
    auto *definition = new IrNodeDefinition(buildToken(*form.child("name")));

    definition->setIsInline(form.child("inline")->flag(1));

    const std::string &visibility = form.child("vis")->items[1].atom;
    if (visibility == "public") definition->setVisibility(IrVisibility::Public);
    else if (visibility == "private") definition->setVisibility(IrVisibility::Private);
    else definition->setVisibility(IrVisibility::Default);

    if (const sx::Node *builtin = form.child("builtin")) {
        definition->setBuiltinName(buildToken(*builtin));
        definition->setDefinesBuiltin(true);
    }
    else {
        definition->setDefinesBuiltin(false);
    }

    if (const sx::Node *scope = form.child("scope")) {
        definition->setScopeToken(buildToken(*scope));
    }

    auto *ports = new IrAttributeDefinitionList;
    for (const sx::Node &port : form.child("ports")->items) {
        if (port.isList && port.head() == "port") ports->addDefinition(buildPort(port));
    }
    definition->setAttributeDefinitionList(ports);

    if (const sx::Node *body = form.child("body")) {
        auto *list = new IrNodeList;
        for (const sx::Node &node : body->items) {
            if (node.isList && node.head() == "node") list->add(buildNode(node));
        }
        definition->setBody(list);
    }
    else {
        definition->setBody(nullptr);
    }

    // `immediate_node_definition: AUTO specific_node_definition` instantiates
    // the node *before* the statement registers the definition.
    if (form.child("auto")->flag(1)) {
        auto *instance =
            new IrNode(*definition->getNameToken(), definition, new IrAttributeList);
        unit.addNode(instance);
    }

    unit.addNodeDefinition(definition);
}

static IrImportStatement *buildImport(const sx::Node &form) {
    auto *statement = new IrImportStatement(buildToken(*form.child("lib")));

    const std::string &visibility = form.child("vis")->items[1].atom;
    if (visibility == "public") statement->setVisibility(IrVisibility::Public);
    else if (visibility == "private") statement->setVisibility(IrVisibility::Private);
    else statement->setVisibility(IrVisibility::Default);

    if (const sx::Node *shortName = form.child("short")) {
        statement->setShortName(buildToken(*shortName));
    }

    return statement;
}

static IrCompilationUnit *buildUnit(const sx::Node &form) {
    auto *unit = new IrCompilationUnit;
    unit->setPath(piranha::IrPath(form.items[1].atom));

    for (const sx::Node &statement : form.items) {
        if (!statement.isList) continue;

        if (statement.head() == "import") {
            unit->addImportStatement(buildImport(statement));
        }
        else if (statement.head() == "def") {
            buildDefinition(statement, *unit);
        }
        else if (statement.head() == "node") {
            unit->addNode(buildNode(statement));
        }
    }

    return unit;
}

// --------------------------------------------------------------- IR printer

static void printValue(std::string &out, IrValue *value);

static void printToken(std::string &out, const piranha::IrTokenInfo &token, const std::string &data) {
    out += "(tok ";
    sx::writeString(out, data);
    out += token.specified ? " 1)" : " 0)";
}

static void printNode(std::string &out, IrNode *node) {
    out += "(node (type ";
    printToken(out, node->getTypeToken(), node->getType());
    out += ") (ns ";
    printToken(out, node->m_library, node->m_library.data);
    out += ") (name ";
    printToken(out, node->getNameToken(), node->getName());
    out += ") (attrs";

    IrAttributeList *attributes = node->getAttributes();
    if (attributes != nullptr) {
        for (int i = 0; i < attributes->getAttributeCount(); ++i) {
            IrAttribute *attribute = attributes->getAttribute(i);
            out += " (attr ";
            printToken(out, attribute->m_name, attribute->getName());
            out += ' ';
            printValue(out, attribute->getValue());
            out += ')';
        }
    }

    out += "))";
}

static void printValue(std::string &out, IrValue *value) {
    char buffer[64];

    switch (value->getType()) {
        case IrValue::ValueType::ConstantInt:
            out += "(int ";
            out += std::to_string(static_cast<IrValueInt *>(value)->getValue());
            out += ')';
            return;

        case IrValue::ValueType::ConstantFloat:
            std::snprintf(buffer, sizeof(buffer), "%.17g",
                static_cast<IrValueFloat *>(value)->getValue());
            out += "(float ";
            out += buffer;
            out += ')';
            return;

        case IrValue::ValueType::ConstantBool:
            out += static_cast<IrValueBool *>(value)->getValue() ? "(bool 1)" : "(bool 0)";
            return;

        case IrValue::ValueType::ConstantString:
            out += "(str ";
            sx::writeString(out, static_cast<IrValueString *>(value)->getValue());
            out += ')';
            return;

        case IrValue::ValueType::ConstantLabel: {
            // `IrValueConstant::useToken` only registers the token for error
            // reporting (`m_token` stays default-constructed), so the label's
            // own flags are not recoverable. A label is always a scanner token,
            // i.e. always specified -- the dump format relies on that.
            out += "(label ";
            printToken(out, piranha::IrTokenInfo(1, 1, 1, 1),
                static_cast<IrValueLabel *>(value)->getValue());
            out += ')';
            return;
        }

        case IrValue::ValueType::NodeReference:
            out += "(nref ";
            printNode(out, static_cast<IrValueNodeRef *>(value)->getValue());
            out += ')';
            return;

        case IrValue::ValueType::BinaryOperation: {
            auto *binary = static_cast<IrBinaryOperator *>(value);
            const char *name = "dot";
            switch (binary->getOperator()) {
                case IrBinaryOperator::Operator::Add: name = "add"; break;
                case IrBinaryOperator::Operator::Sub: name = "sub"; break;
                case IrBinaryOperator::Operator::Mul: name = "mul"; break;
                case IrBinaryOperator::Operator::Div: name = "div"; break;
                default: break;
            }
            out += "(bin ";
            out += name;
            out += ' ';
            printValue(out, binary->getLeft());
            out += ' ';
            printValue(out, binary->getRight());
            out += ')';
            return;
        }

        case IrValue::ValueType::UnaryOperation: {
            auto *unary = static_cast<IrUnaryOperator *>(value);
            const char *name = "default";
            switch (unary->getOperator()) {
                case IrUnaryOperator::Operator::NumericNegate: name = "neg"; break;
                case IrUnaryOperator::Operator::BoolNegate: name = "not"; break;
                case IrUnaryOperator::Operator::Positive: name = "pos"; break;
                default: break;
            }
            out += "(un ";
            out += name;
            out += ' ';
            printValue(out, unary->getOperand());
            out += ')';
            return;
        }

        default:
            out += "(unknown)";
            return;
    }
}

static void printPort(std::string &out, IrAttributeDefinition *port) {
    out += "(port (dir ";
    out += port->getDirection() == IrAttributeDefinition::Direction::Input ? "input"
         : port->getDirection() == IrAttributeDefinition::Direction::Output ? "output"
         : port->getDirection() == IrAttributeDefinition::Direction::Modify ? "modify"
         : "toggle";
    out += ") (name ";
    printToken(out, *port->getNameToken(), port->getName());
    out += port->isAlias() ? ") (alias 1)" : ") (alias 0)";

    if (const piranha::IrTokenInfo *aliasToken = port->getAliasToken()) {
        out += " (alias_tok ";
        printToken(out, *aliasToken, aliasToken->specified ? "alias" : "");
        out += ')';
    }

    if (port->m_typeInfo.data[0].specified || port->m_typeInfo.data[1].specified) {
        out += " (type (ns ";
        printToken(out, port->m_typeInfo.data[0], port->m_typeInfo.data[0].data);
        out += ") (nm ";
        printToken(out, port->m_typeInfo.data[1], port->m_typeInfo.data[1].data);
        out += "))";
    }

    if (IrValue *defaultValue = port->getDefaultValue()) {
        out += " (default ";
        printValue(out, defaultValue);
        out += ')';
    }

    out += ')';
}

static void printDefinition(std::string &out, IrNodeDefinition *definition, bool isAuto) {
    out += "(def (name ";
    printToken(out, *definition->getNameToken(), definition->getName());
    out += definition->isInline() ? ") (inline 1)" : ") (inline 0)";
    out += " (vis ";
    out += definition->getVisibility() == IrVisibility::Public ? "public"
         : definition->getVisibility() == IrVisibility::Private ? "private"
         : "default";
    out += isAuto ? ") (auto 1)" : ") (auto 0)";

    if (definition->isBuiltin()) {
        out += " (builtin ";
        printToken(out, *definition->getBuiltinNameToken(), definition->getBuiltinName());
        out += ')';
    }

    if (definition->getScopeToken()->specified) {
        out += " (scope ";
        printToken(out, *definition->getScopeToken(), definition->m_scopeToken.data);
        out += ')';
    }

    out += " (ports";
    const IrAttributeDefinitionList *ports = definition->getAttributeDefinitionList();
    if (ports != nullptr) {
        for (int i = 0; i < ports->getDefinitionCount(); ++i) {
            out += ' ';
            printPort(out, ports->getDefinition(i));
        }
    }
    out += ')';

    if (IrNodeList *body = definition->getBody()) {
        out += " (body";
        for (int i = 0; i < body->getItemCount(); ++i) {
            out += ' ';
            printNode(out, body->getItem(i));
        }
        out += ')';
    }

    out += ')';
}

static void printUnit(std::string &out, IrCompilationUnit *unit) {
    out += "(unit ";
    sx::writeString(out, unit->getPath().toString());
    out += '\n';

    for (int i = 0; i < unit->getImportStatementCount(); ++i) {
        IrImportStatement *statement = unit->getImportStatement(i);
        out += "(import (vis ";
        out += statement->getVisibility() == IrVisibility::Public ? "public"
             : statement->getVisibility() == IrVisibility::Private ? "private"
             : "default";
        out += ") (lib ";
        printToken(out, piranha::IrTokenInfo(1, 1, 1, 1), statement->getLibName());
        out += ')';
        if (statement->hasShortName()) {
            out += " (short ";
            printToken(out, piranha::IrTokenInfo(1, 1, 1, 1), statement->getShortName());
            out += ')';
        }
        out += ")\n";
    }

    // `auto node ...` hides its instance: the node the parser adds points back
    // at its definition (`IrNode::getDefinition`), which is the only trace the
    // flag leaves on the IR.
    std::set<IrNodeDefinition *> autoDefinitions;
    for (int i = 0; i < unit->getNodeCount(); ++i) {
        if (IrNodeDefinition *definition = unit->getNode(i)->getDefinition()) {
            autoDefinitions.insert(definition);
        }
    }

    for (int i = 0; i < unit->getNodeDefinitionCount(); ++i) {
        IrNodeDefinition *definition = unit->getNodeDefinition(i);
        out += "  ";
        printDefinition(out, definition, autoDefinitions.count(definition) != 0);
        out += '\n';
    }

    for (int i = 0; i < unit->getNodeCount(); ++i) {
        IrNode *node = unit->getNode(i);
        if (node->getDefinition() != nullptr) continue;

        out += "  ";
        printNode(out, node);
        out += '\n';
    }

    out += ")\n";
}

// ------------------------------------------------------------- compile/run

/// `piranha::Compiler` with the parser replaced by IR injection.
class InjectedCompiler : public piranha::Compiler {
public:
    explicit InjectedCompiler(const piranha::LanguageRules *rules)
        : piranha::Compiler(rules) { /* void */ }

    void addUnit(IrCompilationUnit *unit) {
        m_units.push_back(unit);
    }

    IrCompilationUnit *findUnit(const piranha::IrPath &path) {
        const std::string key = path.toString();
        for (IrCompilationUnit *unit : m_units) {
            if (unit->getPath().toString() == key) return unit;
        }

        // Paths are compared by the original through filesystem equivalence;
        // fall back to a canonical comparison for the injected units.
        std::error_code ec;
        const auto wanted = std::filesystem::weakly_canonical(key, ec);
        if (ec) return nullptr;

        for (IrCompilationUnit *unit : m_units) {
            const auto candidate =
                std::filesystem::weakly_canonical(unit->getPath().toString(), ec);
            if (!ec && candidate == wanted) return unit;
        }

        return nullptr;
    }

    piranha::ErrorList *errors() { return &m_errorList; }
    const piranha::LanguageRules *languageRules() const { return m_rules; }

    void runResolve() { resolve(); }
    void runValidate() { validate(); }
};

static std::string readFile(const std::string &path) {
    std::ifstream stream(path, std::ios::binary);
    if (!stream.good()) {
        std::fprintf(stderr, "cannot open '%s'\n", path.c_str());
        std::exit(1);
    }

    std::string text((std::istreambuf_iterator<char>(stream)), std::istreambuf_iterator<char>());
    return text;
}

struct World {
    piranha::NodeProgram program;
    std::vector<IrCompilationUnit *> units;
    InjectedCompiler *compiler = nullptr;
};

/// Mirrors `Compiler::analyze` for units that already exist as objects.
static IrCompilationUnit *analyze(
    InjectedCompiler &compiler,
    const piranha::IrPath &scriptPath,
    std::map<std::string, IrCompilationUnit *> &units)
{
    // `Compiler::getUnit` compares paths with `std::filesystem::equivalent`, so
    // two spellings of the same file are one unit. Canonicalising the key does
    // the same and keeps a cyclic import graph (objects <-> actions) from
    // growing a `..` chain until the path stops fitting in a syscall.
    std::error_code ec;
    const std::string key =
        std::filesystem::weakly_canonical(scriptPath.toString(), ec).string();

    auto cached = units.find(key);
    if (cached != units.end()) return cached->second;

    IrCompilationUnit *resolved = compiler.findUnit(piranha::IrPath(key));
    if (resolved == nullptr) {
        std::fprintf(stderr, "no IR dumped for '%s'\n", key.c_str());
        std::exit(1);
    }

    units[key] = resolved;

    piranha::IrPath canonical(key);
    piranha::IrPath rootDir("");
    canonical.getParentPath(&rootDir);

    for (int i = 0; i < resolved->getImportStatementCount(); ++i) {
        IrImportStatement *statement = resolved->getImportStatement(i);
        std::string libName = statement->getLibName();
        if (libName.size() < 3 || libName.compare(libName.size() - 3, 3, ".mr") != 0) {
            libName += ".mr";
        }

        piranha::IrPath importPath(libName);
        piranha::IrPath fullImportPath =
            importPath.isAbsolute() ? importPath : rootDir.append(importPath);

        if (!fullImportPath.exists()) {
            piranha::IrPath resolvedPath("");
            if (!compiler.resolvePath(importPath, &resolvedPath)) {
                std::fprintf(stderr, "%s: cannot resolve import '%s'\n",
                    key.c_str(), libName.c_str());
                std::exit(147);
            }
            fullImportPath = resolvedPath;
        }

        IrCompilationUnit *imported = analyze(compiler, fullImportPath, units);
        statement->setUnit(imported);
        resolved->addDependency(imported);
    }

    return resolved;
}

static void emitValue(const char *tag, const char *name, double value) {
    unsigned long long bits;
    std::memcpy(&bits, &value, sizeof(bits));
    std::printf("%s,%s,%016llx\n", tag, name, bits);
}

static void emitText(const char *tag, const char *name, const std::string &value) {
    std::printf("%s,%s,%s\n", tag, name, value.c_str());
}

static void emitCount(const char *tag, const char *name, int value) {
    std::printf("%s,%s,%d\n", tag, name, value);
}

/// Everything the script decided about the engine, read back off the finished
/// `Engine` the same way the application reads it.
static void dumpEngine(Engine *engine) {
    emitText("E", "name", engine->getName());
    emitCount("E", "cylinder_banks", engine->getCylinderBankCount());
    emitCount("E", "cylinder_count", engine->getCylinderCount());
    emitCount("E", "crankshaft_count", engine->getCrankshaftCount());
    emitCount("E", "exhaust_system_count", engine->getExhaustSystemCount());
    emitCount("E", "intake_count", engine->getIntakeCount());
    emitValue("E", "displacement", engine->getDisplacement());
    emitValue("E", "redline", engine->getRedline());
    emitValue("E", "starter_torque", engine->getStarterTorque());
    emitValue("E", "starter_speed", engine->getStarterSpeed());
    emitValue("E", "dyno_min_speed", engine->getDynoMinSpeed());
    emitValue("E", "dyno_max_speed", engine->getDynoMaxSpeed());
    emitValue("E", "dyno_hold_step", engine->getDynoHoldStep());
    emitValue("E", "simulation_frequency", engine->getSimulationFrequency());
    emitValue("E", "initial_noise", engine->getInitialNoise());
    emitValue("E", "initial_jitter", engine->getInitialJitter());
    emitValue("E", "initial_high_frequency_gain", engine->getInitialHighFrequencyGain());

    Fuel *fuel = engine->getFuel();
    emitValue("E", "fuel_molecular_mass", fuel->getMolecularMass());
    emitValue("E", "fuel_energy_density", fuel->getEnergyDensity());
    emitValue("E", "fuel_density", fuel->getDensity());
    emitValue("E", "fuel_molecular_afr", fuel->getMolecularAfr());
    emitValue("E", "fuel_max_burning_efficiency", fuel->getMaxBurningEfficiency());
    emitValue("E", "fuel_burning_efficiency_randomness", fuel->getBurningEfficiencyRandomness());
    emitValue("E", "fuel_low_efficiency_attenuation", fuel->getLowEfficiencyAttenuation());
    emitValue("E", "fuel_max_turbulence_effect", fuel->getMaxTurbulenceEffect());
    emitValue("E", "fuel_max_dilution_effect", fuel->getMaxDilutionEffect());

    emitValue("E", "ignition_advance",
        engine->getIgnitionModule()->getTimingAdvance());

    for (int i = 0; i < engine->getCrankshaftCount(); ++i) {
        Crankshaft *crank = engine->getCrankshaft(i);
        char name[64];
        std::snprintf(name, sizeof(name), "crank%d_throw", i);
        emitValue("E", name, crank->getThrow());
        std::snprintf(name, sizeof(name), "crank%d_moment_of_inertia", i);
        emitValue("E", name, crank->getMomentOfInertia());
        std::snprintf(name, sizeof(name), "crank%d_flywheel_mass", i);
        emitValue("E", name, crank->getFlywheelMass());
        std::snprintf(name, sizeof(name), "crank%d_friction_torque", i);
        emitValue("E", name, crank->getFrictionTorque());
        std::snprintf(name, sizeof(name), "crank%d_rod_journals", i);
        emitCount("E", name, crank->getRodJournalCount());
    }

    for (int i = 0; i < engine->getCylinderCount(); ++i) {
        Piston *piston = engine->getPiston(i);
        ConnectingRod *rod = engine->getConnectingRod(i);
        CombustionChamber *chamber = engine->getChamber(i);
        char name[64];

        std::snprintf(name, sizeof(name), "cyl%d_piston_displacement", i);
        emitValue("E", name, piston->getDisplacement());
        std::snprintf(name, sizeof(name), "cyl%d_piston_mass", i);
        emitValue("E", name, piston->getMass());
        std::snprintf(name, sizeof(name), "cyl%d_piston_compression_height", i);
        emitValue("E", name, piston->getCompressionHeight());
        std::snprintf(name, sizeof(name), "cyl%d_piston_wrist_pin", i);
        emitValue("E", name, piston->getWristPinLocation());
        std::snprintf(name, sizeof(name), "cyl%d_piston_blowby_k", i);
        emitValue("E", name, piston->getBlowbyK());

        std::snprintf(name, sizeof(name), "cyl%d_rod_length", i);
        emitValue("E", name, rod->getLength());
        std::snprintf(name, sizeof(name), "cyl%d_rod_mass", i);
        emitValue("E", name, rod->getMass());
        std::snprintf(name, sizeof(name), "cyl%d_rod_moment_of_inertia", i);
        emitValue("E", name, rod->getMomentOfInertia());
        std::snprintf(name, sizeof(name), "cyl%d_rod_center_of_mass", i);
        emitValue("E", name, rod->getCenterOfMass());

        std::snprintf(name, sizeof(name), "cyl%d_chamber_volume", i);
        emitValue("E", name, chamber->getCylinderHead()->getCombustionChamberVolume());
        std::snprintf(name, sizeof(name), "cyl%d_head_intake_runner_volume", i);
        emitValue("E", name, chamber->getCylinderHead()->getIntakeRunnerVolume());
        std::snprintf(name, sizeof(name), "cyl%d_head_intake_runner_area", i);
        emitValue("E", name, chamber->getCylinderHead()->getIntakeRunnerCrossSectionArea());
        std::snprintf(name, sizeof(name), "cyl%d_head_exhaust_runner_volume", i);
        emitValue("E", name, chamber->getCylinderHead()->getExhaustRunnerVolume());
        std::snprintf(name, sizeof(name), "cyl%d_head_exhaust_runner_area", i);
        emitValue("E", name, chamber->getCylinderHead()->getExhaustRunnerCrossSectionArea());
    }

    for (int i = 0; i < engine->getIntakeCount(); ++i) {
        Intake *intake = engine->getIntake(i);
        char name[64];
        std::snprintf(name, sizeof(name), "intake%d_runner_length", i);
        emitValue("E", name, intake->getRunnerLength());
        std::snprintf(name, sizeof(name), "intake%d_plenum_area", i);
        emitValue("E", name, intake->getPlenumCrossSectionArea());
        std::snprintf(name, sizeof(name), "intake%d_velocity_decay", i);
        emitValue("E", name, intake->getVelocityDecay());
    }

    for (int i = 0; i < engine->getExhaustSystemCount(); ++i) {
        ExhaustSystem *exhaust = engine->getExhaustSystem(i);
        char name[64];
        std::snprintf(name, sizeof(name), "exhaust%d_length", i);
        emitValue("E", name, exhaust->getLength());
        std::snprintf(name, sizeof(name), "exhaust%d_primary_tube_length", i);
        emitValue("E", name, exhaust->getPrimaryTubeLength());
        std::snprintf(name, sizeof(name), "exhaust%d_collector_area", i);
        emitValue("E", name, exhaust->getCollectorCrossSectionArea());
        std::snprintf(name, sizeof(name), "exhaust%d_velocity_decay", i);
        emitValue("E", name, exhaust->getVelocityDecay());
        std::snprintf(name, sizeof(name), "exhaust%d_audio_volume", i);
        emitValue("E", name, exhaust->getAudioVolume());
    }
}

/// Builds the default vehicle and transmission the application falls back to
/// when the script does not create them (`EngineSimApplication::loadScript`).
static void loadEngineAndTrace(Engine *engine, int stepCount) {
    Vehicle::Parameters vehParams;
    vehParams.mass = units::mass(1597, units::kg);
    vehParams.diffRatio = 3.42;
    vehParams.tireRadius = units::distance(10, units::inch);
    vehParams.dragCoefficient = 0.25;
    vehParams.crossSectionArea =
        units::distance(6.0, units::foot) * units::distance(6.0, units::foot);
    vehParams.rollingResistance = 2000.0;

    auto *vehicle = new Vehicle;
    vehicle->initialize(vehParams);

    const double gearRatios[] = { 2.97, 2.07, 1.43, 1.00, 0.84, 0.56 };
    Transmission::Parameters tParams;
    tParams.GearCount = 6;
    tParams.GearRatios = gearRatios;
    tParams.MaxClutchTorque = units::torque(1000.0, units::ft_lb);

    auto *transmission = new Transmission;
    transmission->initialize(tParams);

    auto *simulator = static_cast<PhysicsSimulator *>(
        engine->createSimulator(vehicle, transmission));

    engine->calculateDisplacement();
    simulator->setSimulationFrequency(engine->getSimulationFrequency());

    Synthesizer::AudioParameters audioParams = simulator->synthesizer().getAudioParameters();
    audioParams.inputSampleNoise = static_cast<float>(engine->getInitialJitter());
    audioParams.airNoise = static_cast<float>(engine->getInitialNoise());
    audioParams.dF_F_mix = static_cast<float>(engine->getInitialHighFrequencyGain());
    simulator->synthesizer().setAudioParameters(audioParams);

    install_impulse_response(simulator, engine);

    // The application only spins the starter while [S] is held; the harness
    // holds it so the engine actually runs, as `core_reference` does.
    simulator->m_starterMotor.m_enabled = true;

    dumpEngine(engine);
    traceFrames(simulator, engine, stepCount);
}

int main(int argc, char **argv) {
    if (argc < 3) {
        std::fprintf(stderr,
            "usage: script_reference --ir    <dump.txt>\n"
            "       script_reference --check <root.mr> <dump.txt>\n"
            "       script_reference --run   <root.mr> <dump.txt> [steps]\n"
            "       script_reference --resolved <root.mr> <dump.txt>\n"
            "       script_reference --errors <root.mr> <dump.txt>\n");
        return 2;
    }

    const std::string mode = argv[1];
    const std::string dumpPath = (mode == "--ir") ? argv[2] : argv[3];

    const std::string dumpText = readFile(dumpPath);
    sx::Reader reader(dumpText);
    std::vector<sx::Node> forms;
    while (!reader.eof()) {
        forms.push_back(reader.read());
    }

    std::vector<IrCompilationUnit *> units;
    for (const sx::Node &form : forms) {
        if (form.isList && form.head() == "unit") units.push_back(buildUnit(form));
    }

    if (mode == "--ir") {
        std::string out;
        for (IrCompilationUnit *unit : units) printUnit(out, unit);
        std::fwrite(out.data(), 1, out.size(), stdout);
        return 0;
    }

    if ((mode != "--run" && mode != "--check" && mode != "--resolved"
            && mode != "--errors") || argc < 4) {
        std::fprintf(stderr, "unknown mode '%s'\n", mode.c_str());
        return 2;
    }

    const int stepCount = (argc > 4) ? std::atoi(argv[4]) : 200;

    // `es_script::LanguageRules::initialize` registers the built-in node types;
    // `es_script::Compiler::initialize` would do this but also owns the parser.
    // `PARITY_RULES=test` swaps in the rules the original's own test suite uses,
    // whose fixtures live under `piranha/test/sdl/`.
    static es_script::LanguageRules esRules;
    static TestRules testRules;

    const char *rulesChoice = std::getenv("PARITY_RULES");
    const bool useTestRules = (rulesChoice != nullptr && std::strcmp(rulesChoice, "test") == 0);

    piranha::LanguageRules *rules = &esRules;
    if (useTestRules) {
        testRules.initialize();
        rules = &testRules;
    }
    else {
        esRules.initialize();
    }


    auto *compiler = new InjectedCompiler(rules);
    // Same search paths as `es_script::Compiler::initialize`.
    compiler->addSearchPath(piranha::IrPath("../../es/"));
    compiler->addSearchPath(piranha::IrPath("../es/"));
    compiler->addSearchPath(piranha::IrPath("es/"));
    compiler->setFileExtension(".mr");

    piranha::ErrorList errorList;
    for (IrCompilationUnit *unit : units) {
        unit->setRules(rules);
        unit->setErrorList(&errorList);
        // `Compiler::analyze` pushes every unit it parses; the injected units
        // take that place so `resolve()` and `validate()` see the same set.
        compiler->addUnit(unit);
    }

    std::map<std::string, IrCompilationUnit *> analyzed;
    piranha::IrPath rootPath(argv[2]);
    IrCompilationUnit *root = analyze(*compiler, rootPath, analyzed);


    compiler->runResolve();
    compiler->runValidate();

    if (mode == "--errors") {
        // The error catalogue the Rust side has to reproduce, as one
        // `<stage><code>` line per error in discovery order.
        //
        // `PARITY_LINES=1` appends the source line, which is what the test
        // suite asserts (`findError(list, code, 18)`).
        const bool withLines = std::getenv("PARITY_LINES") != nullptr;

        for (int i = 0; i < errorList.getErrorCount(); ++i) {
            const piranha::CompilationError *error = errorList.getCompilationError(i);
            const piranha::ErrorCode_struct &code = error->getErrorCode();

            if (withLines) {
                std::printf("%s%s,%d\n", code.stage.c_str(), code.code.c_str(),
                    error->getErrorLocation()->lineStart);
            }
            else {
                std::printf("%s%s\n", code.stage.c_str(), code.code.c_str());
            }
        }

        return errorList.getErrorCount() > 0 ? 1 : 0;
    }

    if (errorList.getErrorCount() > 0) {
        for (int i = 0; i < errorList.getErrorCount(); ++i) {
            const piranha::CompilationError *error = errorList.getCompilationError(i);
            const piranha::ErrorCode_struct &code = error->getErrorCode();
            std::fprintf(stderr, "error %s%s (line %d): %s\n",
                code.stage.c_str(), code.code.c_str(),
                error->getErrorLocation()->lineStart, code.info.c_str());
        }
        return 1;
    }

    if (mode == "--resolved") {
        // What resolution produced, in analyze order: per unit, each node
        // definition's ports (declared direction/alias, the definition the type
        // resolved to, and the channel type that came out of it), then each
        // unit-level node's attributes and the port each resolved to.
        //
        // This file is the target the Rust resolution and type checking have to
        // reproduce; nothing else records what `resolveDefinitions` +
        // `checkTypes` actually decided.
        auto directionName = [](IrAttributeDefinition *definition) -> const char * {
            switch (definition->getDirection()) {
            case IrAttributeDefinition::Direction::Input: return "input";
            case IrAttributeDefinition::Direction::Output: return "output";
            case IrAttributeDefinition::Direction::Modify: return "modify";
            case IrAttributeDefinition::Direction::Toggle: return "toggle";
            default: return "?";
            }
        };

        auto channelTypeName = [](const piranha::ChannelType *type) -> std::string {
            return (type != nullptr) ? std::string(type->getType()) : std::string("-");
        };

        for (IrCompilationUnit *unit : units) {
            const std::string path = unit->getPath().toString();

            std::printf("U,%s,%d,%d\n", path.c_str(), unit->getNodeCount(),
                unit->getNodeDefinitionCount());

            const int definitionCount = unit->getNodeDefinitionCount();
            for (int d = 0; d < definitionCount; ++d) {
                IrNodeDefinition *definition = unit->getNodeDefinition(d);
                IrAttributeDefinitionList *ports =
                    definition->getAttributeDefinitionList();

                if (ports == nullptr) continue;

                const int portCount = ports->getDefinitionCount();
                for (int p = 0; p < portCount; ++p) {
                    IrAttributeDefinition *port = ports->getDefinition(p);
                    IrNodeDefinition *typeDefinition = port->getTypeDefinition();

                    std::printf("P,%s,%d,%s,%d,%s,%s,%d,%s,%s\n",
                        path.c_str(), d, definition->getName().c_str(),
                        p, port->getName().c_str(), directionName(port),
                        port->isAlias() ? 1 : 0,
                        (typeDefinition != nullptr)
                            ? typeDefinition->getName().c_str() : "-",
                        channelTypeName(port->getImmediateChannelType()).c_str());
                }
            }

            const int nodeCount = unit->getNodeCount();
            for (int i = 0; i < nodeCount; ++i) {
                IrNode *node = unit->getNode(i);
                const std::string symbol = node->getName();
                IrAttributeList *attributes = node->getAttributes();

                if (attributes == nullptr) {
                    std::printf("A,%s,%d,%s,-,-,-\n", path.c_str(), i,
                        symbol.c_str());
                    continue;
                }

                const int attributeCount = attributes->getAttributeCount();
                for (int j = 0; j < attributeCount; ++j) {
                    IrAttribute *attribute = attributes->getAttribute(j);
                    IrAttributeDefinition *resolved =
                        attribute->getAttributeDefinition();

                    std::printf("A,%s,%d,%s,%d,%s,%s,%s\n", path.c_str(), i,
                        symbol.c_str(), j, attribute->getName().c_str(),
                        (resolved != nullptr) ? resolved->getName().c_str() : "-",
                        (resolved != nullptr) ? directionName(resolved) : "-");
                }
            }
        }

        return 0;
    }

    if (mode == "--check") {
        std::printf("OK %s (%d units)\n", argv[2], (int)analyzed.size());
        return 0;
    }

    piranha::NodeProgram program;
    root->build(&program);
    program.initialize();
    if (!program.execute()) {
        std::fprintf(stderr, "script execution failed\n");
        return 1;
    }

    Engine *engine = es_script::Compiler::output()->engine;
    if (engine == nullptr) {
        std::fprintf(stderr, "the script did not create an engine\n");
        return 1;
    }

    std::printf("-- engine --\n");
    loadEngineAndTrace(engine, stepCount);

    return 0;
}
