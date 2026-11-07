#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's JS analysis (and its HTML's scripts'), as indexing's
# js-analyze.sh and html-analyze.sh make the tree's (see js-analyze.js), for
# its semantic delta (see tools/src/try_delta.rs), whose compilation analyzes
# only its C++: of the files in CHANGED (a list of paths, ex: those the push's
# semantic analysis builds) at FILES_ROOT (its head's checkout) which the tree
# lists as JS or HTML files (INDEX_ROOT's js-files and html-files, which its
# repo_files filters made), or which it adds with JS's or HTML's extensions
# (see find-repo-files.py, but not the extensions C++ shares, like .inc),
# into OUT_DIR/{path}, but those OUT_DIR has already (ex: a compilation's).
# Each is numbered as the tree's list numbers it (its line), or past the lists
# for a file it adds, since the symbols of a file's local variables are by its
# number, so that those of its code the push doesn't change are the indexed
# revision's.  Without a JS shell (`js`), it says so and analyzes nothing.
#
# TESTS_ROOT, if set, is the tree's tests' root (its wpt_root), whose files'
# custom properties are each document's own, as the tree's indexing has them
# (see js-analyze.sh).
#
# Usage: try-analyze-js.sh INDEX_ROOT FILES_ROOT CHANGED OUT_DIR

if [ $# -ne 4 ]; then
    echo "Usage: try-analyze-js.sh INDEX_ROOT FILES_ROOT CHANGED OUT_DIR" 1>&2
    exit 1
fi

INDEX_ROOT=$1
FILES_ROOT=$2
CHANGED=$3
OUT_DIR=$4

if ! command -v js > /dev/null; then
    echo "WARNING: There's no JS shell (js), so the push's JS isn't analyzed."
    exit 0
fi
# (Required by std::wcsrtombs, used in os.file.redirect; see js-analyze.sh.)
export LC_CTYPE=C.UTF-8
WASM_DIR=${MOZSEARCH_WASM_DIR:-/nix/var/nix/profiles/default/share/wasm-css-analyzer}
TESTS_ROOT=${TESTS_ROOT:--}

LIST=$(mktemp)
trap 'rm -f "$LIST"' EXIT
JS_FILES=$INDEX_ROOT/js-files
HTML_FILES=$INDEX_ROOT/html-files
[ -f "$JS_FILES" ] || JS_FILES=/dev/null
[ -f "$HTML_FILES" ] || HTML_FILES=/dev/null
awk 'FILENAME == ARGV[1] { js[$0] = FNR; next }
     FILENAME == ARGV[2] { html[$0] = FNR; next }
     $0 in js { print js[$0], $0; next }
     $0 in html { print html[$0], $0; next }
     /\.(js|jsm|mjs|html|xhtml|svg)$/ { print 100000000 + (++added), $0 }' \
    "$JS_FILES" "$HTML_FILES" "$CHANGED" |
    while read -r NUMBER FILE; do
        # (Not those it removes, nor those already analyzed.)
        if [ -f "$FILES_ROOT/$FILE" ] && [ ! -e "$OUT_DIR/$FILE" ]; then
            mkdir -p "$(dirname "$OUT_DIR/$FILE")"
            echo "$NUMBER $FILE"
        fi
    done > "$LIST"

echo "Analyzing the push's $(wc -l < "$LIST") JS and HTML files."
if [ -s "$LIST" ]; then
    # (Its pages' inline scripts are js-analyze's, "-", as the push's JS
    # environments aren't analyzed.)
    js -f "$MOZSEARCH_PATH/scripts/js-analyze.js" -- \
        "$MOZSEARCH_PATH" "$FILES_ROOT" "$OUT_DIR" "$WASM_DIR" - "$TESTS_ROOT" < "$LIST"
fi
