#!/usr/bin/env bash
# Records the builtin reference nodes' port lists, for both rule sets.
#
#   ./record_builtin_ports.sh
#
# `IrNodeDefinition::validateBuiltinMappings` compares a `=>` definition's ports
# against the builtin's *reference node*, which is a real `Node` instance built
# by `Rule::generateReference` -- the port list exists only in C++. So it is
# recorded here, once, into `builtin_ports.txt`, and `extract_language_rules.py`
# generates the Rust table from that file. Re-run this when the reference's node
# classes change; `script_reference --builtin-ports` is the dump it drives.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
WORK="${WORK:-/tmp/engine-sim-script-parity}"
OUT="${OUT:-$HERE/builtin_ports.txt}"

"$HERE/build_script_reference.sh" >/dev/null

# The names each rule set registers, read out of the Rust table that the crate's
# own test compares against the `registerBuiltinType` calls.
python3 - "$RUST_DIR/crates/engine-sim-script/src/builtin_types.rs" "$WORK" <<'PY'
import pathlib
import re
import sys

source = pathlib.Path(sys.argv[1]).read_text()
out = pathlib.Path(sys.argv[2])

# `BUILTIN_NODE_TYPES` is a slice (`&[&str]`), the piranha table is an array.
for const, filename in [
    ("BUILTIN_NODE_TYPES", "names_engine_sim.txt"),
    ("PIRANHA_BUILTIN_NODE_TYPES", "names_piranha.txt"),
]:
    match = re.search(rf"pub const {const}: [^=]+= &?\[(.*?)\];", source, re.S)
    assert match, const
    names = re.findall(r'"([^"]+)"', match.group(1))
    assert names, const
    (out / filename).write_text("\n".join(names) + "\n")
    print(f"  {filename}: {len(names)} names")
PY

RAW="$WORK/builtin_ports.raw"
: > "$RAW"

for rules in engine_sim piranha; do
    echo "[$rules]" >> "$RAW"

    if [ "$rules" = "piranha" ]; then
        PARITY_RULES=test "$WORK/script_reference" \
            --builtin-ports "$WORK/names_$rules.txt" >> "$RAW"
    else
        "$WORK/script_reference" --builtin-ports "$WORK/names_$rules.txt" >> "$RAW"
    fi
done

{
    echo "# The builtin reference nodes' ports, recorded from the original."
    echo "#"
    echo '# `IrNodeDefinition::validateBuiltinMappings` resolves a `=>` definition'"'"'s'
    echo "# ports against the builtin's reference node -- a real \`Node\` instance --"
    echo "# so the list exists in C++ only. Rows:"
    echo "#"
    echo "#     builtin,<builtin>"
    echo "#     input,<builtin>,<port>,<modifiesInput>,<enableInput>"
    echo "#     output,<builtin>,<port>,<isAlias>"
    echo "#     absent,<builtin>"
    echo "#"
    echo "# Recorded by \`record_builtin_ports.sh\`; \`extract_language_rules.py\`"
    echo "# generates \`src/language_rules.rs\` from it."
    cat "$RAW"
} > "$OUT"

echo "  $OUT: $(grep -vc '^#' "$OUT") rows"
