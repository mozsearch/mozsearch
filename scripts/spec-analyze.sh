#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 3 ]
then
    echo "Usage: spec-analyze.sh config-repo-path config-file.json tree_name"
    exit 1
fi

CONFIG_REPO=$1
CONFIG_FILE=$2
TREE_NAME=$3

# A tree with web specs lists them in its config directory's specs.toml (ex:
# the wubspecs tree's, which also configures its forest), and their sources get
# the terms they define and their references to them (see
# tools/src/spec_analyze), and the tree's other files their citations of them
# (see tools/src/spec_analyze/citations.rs).  A tree whose `SPEC_` symbols
# are another tree's (its `linked_trees`, ex: firefox-disco's, which are
# wubspecs') gets its files' citations of that tree's specs.
SPECS_TOML=$CONFIG_REPO/$TREE_NAME/specs.toml
if [ -f "$SPECS_TOML" ]; then
    spec-analyze $FILES_ROOT $INDEX_ROOT/analysis $SPECS_TOML $INDEX_ROOT/repo-files
    # (The index's copy is for the rendered views of the specs' sources, see
    # tools/src/rendered_specs, which the web server makes from it, and the
    # source listings' links to them.)
    cp "$SPECS_TOML" $INDEX_ROOT/specs.toml
    exit 0
fi

LINKED_TREE=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].linked_trees.SPEC_ // empty' "$CONFIG_FILE")
if [ -n "$LINKED_TREE" ] && [ -f "$CONFIG_REPO/$LINKED_TREE/specs.toml" ]; then
    # (With the linked tree's files, which were indexed first, for its ecmarkup
    # specs' clauses' titles, which comments cite sections by, and its index,
    # for the anchors its specs define, which citations of anchors their specs
    # don't define can be of another spec's.)
    SPEC_FILES=$(jq -r --arg tree "$LINKED_TREE" '.trees[$tree].files_path // empty' "$CONFIG_FILE")
    SPEC_INDEX=$(jq -r --arg tree "$LINKED_TREE" '.trees[$tree].index_path // empty' "$CONFIG_FILE")
    spec-analyze --citations-only ${SPEC_FILES:+--spec-files "$SPEC_FILES"} \
        ${SPEC_INDEX:+--spec-index "$SPEC_INDEX"} \
        $FILES_ROOT $INDEX_ROOT/analysis \
        "$CONFIG_REPO/$LINKED_TREE/specs.toml" $INDEX_ROOT/repo-files
fi
