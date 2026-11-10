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
# they apply to (ex: Fluent's .ftl files) out of all of the repo's files, and
# documents (Markdown and reStructuredText) get their Sphinx labels and
# references (with the documentation's staged layout, if the tree has one).
# What they define, which uses in any file can name, is saved by file, for the
# tree's try pushes' semantic analyses, which then analyze only their files
# (see try-analyze-rest.sh).
semantic-analyze --save-resolvables $INDEX_ROOT/semantic-resolvables.jsonl \
    $FILES_ROOT $INDEX_ROOT/analysis $INDEX_ROOT/doc-trees.json < $INDEX_ROOT/repo-files
