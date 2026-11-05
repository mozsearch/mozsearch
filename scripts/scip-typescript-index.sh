#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# scip-typescript's index of a tree's TypeScript and JS projects, as a tree's
# indexing (its config's build, for its `typescript` scip subtree) and its try
# pushes' semantic analysis (see try-analyze-typescript.sh) both make it, so
# that an index of only some of its files (ONLY_FILES, a list of their paths
# relative to ROOT; see scip-typescript's --only-files) has the documents a
# full index has of them: with --stable-type-ordering, no limit on the files'
# sizes, and the projects of PROJECTS (a list of their tsconfig.json or
# jsconfig.json files, relative to ROOT), in its order (a file in several
# projects has a document in each, the last of which is its analysis; see
# scip-indexer).
#
# Usage: scip-typescript-index.sh ROOT PROJECTS OUTPUT [ONLY_FILES]

if [ $# -lt 3 ] || [ $# -gt 4 ]; then
    echo "Usage: scip-typescript-index.sh ROOT PROJECTS OUTPUT [ONLY_FILES]" 1>&2
    exit 1
fi

ROOT=$1
PROJECTS=$(realpath "$2")
OUTPUT=$(realpath -m "$3")
ONLY_FILES=${4:+$(realpath "$4")}

mapfile -t PROJECT_LIST < "$PROJECTS"
cd "$ROOT"
scip-typescript index --no-progress-bar --stable-type-ordering --max-file-byte-size 0 \
    ${ONLY_FILES:+--only-files "$ONLY_FILES"} --output "$OUTPUT" "${PROJECT_LIST[@]}"
