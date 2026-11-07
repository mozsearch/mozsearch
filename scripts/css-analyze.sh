#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: css-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# (The custom properties of the tree's tests' files, under its wpt_root, are
# each file's own; "-" if it has none.)
TESTS_ROOT=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].wpt_root // "-"' "$CONFIG_FILE")

# (Their analyses' directories, which a tree's build can have left out; see
# html-analyze.sh.)
(cd $INDEX_ROOT/analysis && xargs -r -d '\n' -a $INDEX_ROOT/css-files dirname | sort -u |
    xargs -r -d '\n' mkdir -p)

cat $INDEX_ROOT/css-files | \
    parallel css-analyze $FILES_ROOT {} "$TESTS_ROOT" ">" $INDEX_ROOT/analysis/{}
echo $?
