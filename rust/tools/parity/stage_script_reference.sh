#!/usr/bin/env bash
# Stage a reference build of the scripting stack (piranha + es_script).
#
# This is the M0 half of the `.mr` interpreter parity work: the upstream
# compiler front end exists only as flex/bison input, and neither tool is
# available here, so the reference is built without a parser and is fed the IR
# that the Rust parser produces (see `script_reference.cpp`).
#
# Layout:
#   $REF_SCRIPT/include -> $REF_CORE/include   (engine headers, already patched)
#   $REF_SCRIPT/dependencies/submodules/piranha  (patched copy)
#   $REF_SCRIPT/scripting                        (copy, unmodified)
#
#   ./stage_script_reference.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
REF_CORE="${REF_CORE:-/tmp/refcore}"
REF_SCRIPT="${REF_SCRIPT:-/tmp/refscript}"
PIRANHA_SRC="$REPO_ROOT/dependencies/submodules/piranha"

if [ ! -f "$PIRANHA_SRC/include/piranha.h" ]; then
    echo "error: the piranha submodule is not checked out." >&2
    echo "       git submodule update --init dependencies/submodules/piranha" >&2
    exit 1
fi

if [ ! -f "$REF_CORE/librefcore.a" ]; then
    "$HERE/stage_reference.sh"
fi

rm -rf "$REF_SCRIPT"
mkdir -p "$REF_SCRIPT/dependencies/submodules"
cp -r "$PIRANHA_SRC" "$REF_SCRIPT/dependencies/submodules/piranha"
cp -r "$REPO_ROOT/scripting" "$REF_SCRIPT/scripting"
ln -sfn "$REF_CORE/include" "$REF_SCRIPT/include"

echo "==> patching piranha (no arithmetic changes)"
python3 - "$REF_SCRIPT/dependencies/submodules/piranha" "$REF_SCRIPT/scripting" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
script_root = pathlib.Path(sys.argv[2])


def rewrite_in(base, path, replacements):
    file = base / path
    text = file.read_text()

    for old, new in replacements:
        if old not in text:
            raise SystemExit(f"patch did not apply to {path}: missing\n{old!r}")
        text = text.replace(old, new)

    file.write_text(text)


def rewrite(path, replacements):
    rewrite_in(root, path, replacements)


# ---------------------------------------------------------------- Boost removal
# `boost::filesystem` is only used to hold and compare paths; `std::filesystem`
# provides the same operations. Nothing here touches the interpreter's values.
rewrite("include/path.h", [
    ("""#include <string>

// Forward Boost declaration
namespace boost {
    namespace filesystem {
        class path;
    } /* namespace filesystem */
} /* namespace boost */
""", """#include <filesystem>
#include <string>
"""),
    ("boost::filesystem::path", "std::filesystem::path"),
])

rewrite("src/path.cpp", [
    ("#include <boost/filesystem.hpp>", "#include <filesystem>\n#include <system_error>"),
    ("boost::filesystem::path", "std::filesystem::path"),
    ("m_path->is_complete()", "m_path->is_absolute()"),
    ("return boost::filesystem::exists(*m_path);", "return std::filesystem::exists(*m_path);"),
    # `equivalent` throws when either path is missing; fall back to a
    # lexical comparison so unit caching behaves like the Boost version.
    ("""bool piranha::Path::operator==(const Path &path) const {
    return boost::filesystem::equivalent(this->getBoostPath(), path.getBoostPath());
}""", """bool piranha::Path::operator==(const Path &path) const {
    std::error_code ec;
    const bool equivalent = std::filesystem::equivalent(
        this->getBoostPath(), path.getBoostPath(), ec);

    if (!ec) return equivalent;

    return this->getBoostPath().lexically_normal() == path.getBoostPath().lexically_normal();
}"""),
])

# --------------------------------------------------- MSVC-only in-class templates
# MSVC allows explicit specialisation of a member template inside the class
# body; GCC does not. Moving them to namespace scope is the standard form and
# changes nothing but where the definition sits.
rewrite("include/language_rules.h", [
    ("""        template<> std::string getLiteralBuiltinName<piranha::native_bool>() const { return *m_literalRules.lookup(LiteralType::Boolean); }
        template<> std::string getLiteralBuiltinName<piranha::native_string>() const  { return *m_literalRules.lookup(LiteralType::String); }
        template<> std::string getLiteralBuiltinName<piranha::native_int>() const  { return *m_literalRules.lookup(LiteralType::Integer); }
        template<> std::string getLiteralBuiltinName<piranha::native_float>() const { return *m_literalRules.lookup(LiteralType::Float); }
""", ""),
    ("""    class LanguageRules {
    public:
        LanguageRules();""", """    class LanguageRules {
    public:
        enum { kLiteralBuiltinSpecializationsMovedOutOfClass = 1 };
        LanguageRules();"""),
])

# Same MSVC-ism inside a class *template*, where an out-of-class member
# specialisation is not expressible: make it a plain overload instead. Overload
# resolution prefers the non-template for `native_string`, which is exactly what
# the specialisation did.
rewrite("include/ir_value_constant.h", [
    ("""        template <>
        const piranha::native_string validateData<piranha::native_string>(const piranha::native_string &data, bool recordErrors) {""",
     """        const piranha::native_string validateData(const piranha::native_string &data, bool recordErrors) {"""),
])

source = (root / "include/language_rules.h").read_text()
marker = "} /* namespace piranha */"
specializations = """
    template <> inline std::string LanguageRules::getLiteralBuiltinName<native_bool>() const {
        return *m_literalRules.lookup(LiteralType::Boolean);
    }

    template <> inline std::string LanguageRules::getLiteralBuiltinName<native_string>() const {
        return *m_literalRules.lookup(LiteralType::String);
    }

    template <> inline std::string LanguageRules::getLiteralBuiltinName<native_int>() const {
        return *m_literalRules.lookup(LiteralType::Integer);
    }

    template <> inline std::string LanguageRules::getLiteralBuiltinName<native_float>() const {
        return *m_literalRules.lookup(LiteralType::Float);
    }

"""
source = source.replace(marker, specializations + marker, 1)
(root / "include/language_rules.h").write_text(source)

# ------------------------------------------------------------ MSVC-only keywords
# `unsigned __int64` is an MSVC extension.
rewrite("include/memory_management.h", [
    ("unsigned __int64", "unsigned long long"),
])

# The engine's channel-type registration uses two more MSVC-only forms: a
# non-dependent `static_assert(false)` (which GCC rejects even in an uninstantiated
# template) and `extern inline` on an explicit specialisation.
rewrite_in(script_root, "include/channel_types.h", [
    ('static_assert(false, "Invalid type lookup");',
     'static_assert(sizeof(Type) == 0, "Invalid type lookup");'),
    ("template <> extern inline const piranha::ChannelType *LookupChannelType<type>()",
     "template <> inline const piranha::ChannelType *LookupChannelType<type>()"),
])

# A member template may not reuse the class template's parameter name.
rewrite_in(script_root, "include/object_reference_node.h", [
    ("""        template <typename Type>
        void overrideType() {
            m_output.overrideType(LookupChannelType<Type>());
        }""", """        template <typename T_OverrideType>
        void overrideType() {
            m_output.overrideType(LookupChannelType<T_OverrideType>());
        }"""),
])

# ------------------------------------------------------ parser-less compilation
# `parseHelper` is the only place the generated scanner/parser is used. The
# reference is fed IR directly, so it is replaced by a stub; `parse()` is never
# called by the harness.
rewrite("src/ir_compilation_unit.cpp", [
    # `scanner.h` used to pull these in transitively.
    ('#include "../include/scanner.h"\n',
     '#include "../include/ir_node_definition.h"\n#include "../include/ir_import_statement.h"\n'
     '#include "../include/compilation_error.h"\n#include "../include/ir_attribute_definition.h"\n'
     '#include "../include/ir_attribute_definition_list.h"\n#include "../include/ir_value.h"\n'),
    ("""    delete FTRACK(m_parser);
    delete FTRACK(m_scanner);
""", """    // The generated parser/scanner are not part of this build.
    m_parser = nullptr;
    m_scanner = nullptr;
"""),
])

source = (root / "src/ir_compilation_unit.cpp").read_text()
start = source.index("piranha::IrCompilationUnit::ParseResult piranha::IrCompilationUnit::parseHelper(")
end = source.index("\n}\n", source.index("else return ParseResult::Success;")) + 3
source = source[:start] + """piranha::IrCompilationUnit::ParseResult piranha::IrCompilationUnit::parseHelper(
    std::istream &stream, IrCompilationUnit *topLevel)
{
    // This reference build has no flex/bison front end: the IR is injected by
    // `script_reference.cpp` instead. Parsing is therefore unavailable.
    (void)stream;
    (void)topLevel;
    return ParseResult::Fail;
}
""" + source[end:]
(root / "src/ir_compilation_unit.cpp").write_text(source)

# --------------------------------------------------------------- path injection
# `parseFile` sets the unit path; injected units need an equivalent setter so
# import resolution and error messages see the right file.
rewrite("include/ir_compilation_unit.h", [
    ("""        void setErrorList(ErrorList *list) { m_errorList = list; }""",
     """        void setErrorList(ErrorList *list) { m_errorList = list; }

        // Reference build: units are built from an injected IR rather than parsed.
        void setPath(const Path &path) { m_path = path; }"""),
])

print("    piranha patched")
PY

CFLAGS=(-std=c++17 -O2 -fpermissive -D__forceinline=inline
        -include cmath -include cstdlib -include cstring
        -I "$REF_SCRIPT/dependencies/submodules"
        -I "$REF_SCRIPT/dependencies/submodules/piranha/include"
        -I "$REF_SCRIPT/scripting/include"
        -I "$REF_CORE/include"
        -I "$REF_CORE/dependencies/submodules/simple-2d-constraint-solver/include"
        -I "$REF_CORE/dependencies/submodules/csv-io/include")

mkdir -p "$REF_SCRIPT/obj"

echo "==> compiling piranha"
for src in "$REF_SCRIPT"/dependencies/submodules/piranha/src/*.cpp; do
    base="$(basename "$src" .cpp)"
    g++ "${CFLAGS[@]}" -c "$src" -o "$REF_SCRIPT/obj/piranha_$base.o"
done

echo "==> compiling es_script"
for src in "$REF_SCRIPT"/scripting/src/*.cpp; do
    base="$(basename "$src" .cpp)"
    g++ "${CFLAGS[@]}" -c "$src" -o "$REF_SCRIPT/obj/es_script_$base.o"
done

echo "==> archiving"
rm -f "$REF_SCRIPT/librefscript.a"
ar rcs "$REF_SCRIPT/librefscript.a" "$REF_SCRIPT"/obj/*.o

echo "scripting reference ready: $REF_SCRIPT/librefscript.a"
