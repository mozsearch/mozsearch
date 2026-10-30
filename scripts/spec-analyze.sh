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
#
# Citations of anchors the specs no longer define (but once did) are marked as
# the history of the specs' tree, if it has one, has them (its cache, which
# the stale-anchors tool updates with the history), and those cited go in
# stale-anchors-cited.json, for the next history update to look up (ex: the
# config repo's upload-after-index scripts upload it, and wubspecs'
# update-history.sh downloads them); see tools/src/spec_analyze/stale.rs.
STALE_CITED=$INDEX_ROOT/stale-anchors-cited.json
#
# MDN's browser-compat-data, if the tree's setup put it in bcd/data.json (ex:
# the config repo's shared/fetch-bcd.sh), gives the symbols of the tree's
# WebIDL files the specs' definitions which they reflect and their MDN pages,
# and a spec tree's definitions their MDN pages (see
# tools/src/spec_analyze/bcd.rs).
BCD_ARGS=()
if [ -f "$INDEX_ROOT/bcd/data.json" ]; then
    BCD_ARGS=(--bcd "$INDEX_ROOT/bcd/data.json")
fi
SPECS_TOML=$CONFIG_REPO/$TREE_NAME/specs.toml
if [ -f "$SPECS_TOML" ]; then
    STALE_ANCHORS=()
    if [ -n "${HISTORY_ROOT:-}" ] && [ -d "$HISTORY_ROOT/timeline" ]; then
        STALE_ANCHORS=(--stale-anchors "$TREE_NAME" "$GIT_ROOT" "$HISTORY_ROOT" "$STALE_CITED")
    fi
    # (Bikeshed's renderings of its specs go in rendered-specs/, for their
    # rendered views; see tools/src/rendered_specs/bikeshed.rs.  And the specs'
    # exported definitions in spec-terms.json, for analyzing their pull
    # requests' changes without the other specs; see analyze-spec-pr.)
    spec-analyze --rendered-specs $INDEX_ROOT/rendered-specs "${STALE_ANCHORS[@]}" "${BCD_ARGS[@]}" \
        --terms-out $INDEX_ROOT/spec-terms.json \
        $FILES_ROOT $INDEX_ROOT/analysis $SPECS_TOML $INDEX_ROOT/repo-files
    # (The index's copy is for the rendered views of the specs' sources, see
    # tools/src/rendered_specs, which the web server makes from it, and the
    # source listings' links to them.)
    cp "$SPECS_TOML" $INDEX_ROOT/specs.toml
    # The excerpts of the specs' rendered views, for search results' lines in
    # their sources (in spec-excerpts/; see build-spec-excerpts).
    echo "Performing spec-analyze::excerpts step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    build-spec-excerpts $CONFIG_FILE $TREE_NAME
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
    SPEC_GIT=$(jq -r --arg tree "$LINKED_TREE" '.trees[$tree].git_path // empty' "$CONFIG_FILE")
    SPEC_HISTORY=$(jq -r --arg tree "$LINKED_TREE" '.trees[$tree].history_path // empty' "$CONFIG_FILE")
    STALE_ANCHORS=()
    if [ -n "$SPEC_INDEX" ] && [ -n "$SPEC_GIT" ] && [ -n "$SPEC_HISTORY" ] && [ -d "$SPEC_HISTORY/timeline" ]; then
        STALE_ANCHORS=(--stale-anchors "$LINKED_TREE" "$SPEC_GIT" "$SPEC_HISTORY" "$STALE_CITED")
    fi
    spec-analyze --citations-only ${SPEC_FILES:+--spec-files "$SPEC_FILES"} \
        ${SPEC_INDEX:+--spec-index "$SPEC_INDEX"} "${STALE_ANCHORS[@]}" "${BCD_ARGS[@]}" \
        $FILES_ROOT $INDEX_ROOT/analysis \
        "$CONFIG_REPO/$LINKED_TREE/specs.toml" $INDEX_ROOT/repo-files
fi
