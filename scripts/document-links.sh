#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: document-links.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# web-platform-tests' pages' and stylesheets' links to the tree's files
# (js-environments.sh's js-environments-links.json), written into their
# analyses (see tools/src/bin/document-links.rs): as uses of the files, and
# as their documents' custom properties' uses of each other's definitions.
# It runs after the pages' and stylesheets' analyses (js-, html- and
# css-analyze, and js-environments-pages.sh's merge).
if [ ! -f "$INDEX_ROOT/js-environments-links.json" ]; then
    echo "There are no web-platform-tests' links."
    exit 0
fi
document-links "$INDEX_ROOT"
