#!/usr/bin/env bash

set -eu
set -o pipefail

# A key of what a tree's history (its token-centric history, or its line
# blame) is derived with, for the test trees whose histories are kept between
# builds rather than built again from scratch (see tests/searchfox/setup and
# tests/searchfox-line-blame/setup, and the Makefile's check-all's
# CLEAN_HISTORY=auto): the hash of the git blob ids, at mozsearch's checkout's
# HEAD (which builds require to be committed), of the source files the history
# tools derive their histories with, and of EXTRA_PATHS (ex: the tree's history
# configuration).  If a history tool's derivation starts depending on another
# module, add it to HISTORY_INPUTS: a change to it that changes histories
# would otherwise go unnoticed by the kept histories (until CI, whose
# histories are always built from scratch, or a CLEAN_HISTORY=1 build).
#
# Usage: history-inputs-key.sh MOZSEARCH_PATH [EXTRA_PATHS...]

MOZSEARCH_PATH=$1
shift

HISTORY_INPUTS=(
    # The history tools.
    tools/src/bin/build-syntax-token-tree.rs
    tools/src/bin/build-timeline-tree.rs
    tools/src/bin/check-token-counts.rs
    tools/src/bin/build-commit-index.rs
    tools/src/bin/build-blame.rs
    scripts/build-history.py
    # What they derive the histories with: tokens and their syntax, the
    # timeline's journals and summaries, the source mapping and git's.
    tools/src/hyperblame
    tools/src/tokenize.rs
    tools/src/tree_sitter_support
    tools/src/languages.rs
    tools/src/file_format
    tools/src/source_mapping.rs
    tools/src/cinnabar.rs
    tools/src/commit_index.rs
    tools/src/blame.rs
    tools/src/fast_import.rs
    tools/src/git_notes.rs
    tools/src/git_ops.rs
    tools/src/history_stop.rs
    tools/src/lib.rs
    # Their dependencies (ex: the tree-sitter grammars' versions), and the
    # grammars mozsearch vendors.
    tools/Cargo.toml
    tools/Cargo.lock
    deps
)

# (Each of HISTORY_INPUTS must exist, so that a module's move can't leave it
# out unnoticed; EXTRA_PATHS needn't.)
paths=()
for path in "${HISTORY_INPUTS[@]}"; do
    if [ ! -e "$MOZSEARCH_PATH/$path" ]; then
        echo "history-inputs-key.sh: $path doesn't exist; update HISTORY_INPUTS." 1>&2
        exit 1
    fi
    paths+=("$path")
done
for path in "$@"; do
    if [ -e "$MOZSEARCH_PATH/$path" ]; then
        paths+=("$path")
    fi
done
git -C "$MOZSEARCH_PATH" ls-tree -r HEAD -- "${paths[@]}" | sha256sum | cut -d' ' -f1
