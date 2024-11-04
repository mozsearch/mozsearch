#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

$MOZSEARCH_PATH/scripts/staticprefs-analyze.py \
    $INDEX_ROOT/staticprefs-files \
    $STATICPREFS_BINDINGS_LOCAL_PATH \
    $FILES_ROOT $INDEX_ROOT/analysis
