#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: semantic-analyze.sh config-file.json tree_name"
    exit 1
fi

# The tree-sitter mappings (tools/languages/semantic_mappings) pick the files
# they apply to (ex: Fluent's .ftl files) out of all of the repo's files.
semantic-analyze $FILES_ROOT $INDEX_ROOT/analysis < $INDEX_ROOT/repo-files
