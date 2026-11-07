#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's stylesheets' analysis, as indexing's css-analyze.sh makes the
# tree's, for its semantic delta (see tools/src/try_delta.rs): of the files in
# CHANGED at FILES_ROOT (its head's checkout) which the tree lists as CSS
# files (INDEX_ROOT's css-files, which its repo_files filters made), or which
# it adds, into OUT_DIR/{path}, but those OUT_DIR has already.  TESTS_ROOT, if
# set, is the tree's tests' root (its wpt_root), whose files' custom
# properties are each file's own, as the tree's are.
#
# Usage: try-analyze-css.sh INDEX_ROOT FILES_ROOT CHANGED OUT_DIR

if [ $# -ne 4 ]; then
    echo "Usage: try-analyze-css.sh INDEX_ROOT FILES_ROOT CHANGED OUT_DIR" 1>&2
    exit 1
fi
INDEX_ROOT=$1
FILES_ROOT=$2
CHANGED=$3
OUT_DIR=$4
TESTS_ROOT=${TESTS_ROOT:--}

CSS_FILES=$INDEX_ROOT/css-files
[ -f "$CSS_FILES" ] || CSS_FILES=/dev/null
LIST=$(mktemp)
trap 'rm -f "$LIST"' EXIT
awk 'FILENAME == ARGV[1] { css[$0] = 1; next }
     $0 in css || /\.css$/ { print }' "$CSS_FILES" "$CHANGED" |
    while read -r FILE; do
        # (Not those it removes, nor those already analyzed.)
        if [ -f "$FILES_ROOT/$FILE" ] && [ ! -e "$OUT_DIR/$FILE" ]; then
            mkdir -p "$(dirname "$OUT_DIR/$FILE")"
            echo "$FILE"
        fi
    done > "$LIST"

echo "Analyzing the push's $(wc -l < "$LIST") CSS files."
while read -r FILE; do
    css-analyze "$FILES_ROOT" "$FILE" "$TESTS_ROOT" > "$OUT_DIR/$FILE"
done < "$LIST"
