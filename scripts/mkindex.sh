#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file> <tree-name>"
    exit 1
fi

CONFIG_REPO=$1
CONFIG_FILE=$2
TREE_NAME=$3

# This script also depends on a `handle_tree_error` function having been defined
# by `load-vars.sh` which will log a warning/error and either return a non-zero
# (failure) exit code if the `on_error` tree mode was "halt" or a zero (success)
# exit code if the mode was "continue".
#
# We annotate a bunch of analysis-related steps with the error handler here
# because they seem potentially prone to breakage due to changes in the tree but
# where the breakage is recoverable in the sense that if we keep going, nothing
# else is likely to break.  It's possible that some of these steps failing will
# in fact turn out to be fatal later on, but we can address that as situations
# arise.

export PYTHONPATH=$MOZSEARCH_PATH/scripts
# This was previously "full" but "1" is much more readable.  Obviously change
# this back if we end up missing things.
#
# Also note that if "parallel" is used, it's necessary to add an arg pair of
# "--env RUST_BACKTRACE" unless using "env_parallel".
export RUST_BACKTRACE=1

date

$MOZSEARCH_PATH/scripts/find-repo-files.py $CONFIG_REPO $CONFIG_FILE $TREE_NAME
$MOZSEARCH_PATH/scripts/mkdirs.sh

date

$MOZSEARCH_PATH/scripts/build.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME

date

export RUST_LOG=info

date

# Transform any .scip files the config `scip_subtrees` setting tells us about.
# This does not generate any .scip files; instead it is assumed that they would
# have been generated in the config's `build` script.  It's also assumed that
# for complicated situations like mozilla-central where merge-analyses may be
# required, that the scripts will handle calling `scip-analyze.sh` or
# `scip-indexer` directly and will not list them in `scip_subtrees`.
$MOZSEARCH_PATH/scripts/scip-analyze.sh \
  "$CONFIG_FILE" \
  "$TREE_NAME" || handle_tree_error "scip-analyze.sh"

date

$MOZSEARCH_PATH/scripts/find-objdir-files.sh
$MOZSEARCH_PATH/scripts/objdir-mkdirs.sh

date

$MOZSEARCH_PATH/scripts/process-chrome-map.py $GIT_ROOT $INDEX_ROOT/aliases/url-map.json $INDEX_ROOT/*.chrome-map.json || handle_tree_error "process-chrome-map.py"

date

$MOZSEARCH_PATH/scripts/js-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "js-analyze.sh"

date

$MOZSEARCH_PATH/scripts/html-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "html-analyze.sh"

date

$MOZSEARCH_PATH/scripts/css-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "css-analyze.sh"

date

$MOZSEARCH_PATH/scripts/idl-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "idl-analyze.sh"

date

$MOZSEARCH_PATH/scripts/ipdl-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "ipdl-analyze.sh"

date

$MOZSEARCH_PATH/scripts/replace-aliases.sh || handle_tree_error "replace-aliases.sh"

date

# crossref failures always need to be fatal because their outputs are required.
$MOZSEARCH_PATH/scripts/crossref.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/output.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME || handle_tree_error "output.sh"

date

$MOZSEARCH_PATH/scripts/build-codesearch.py $CONFIG_FILE $TREE_NAME || handle_tree_error "build-codesearch.py"

date

# This depends on INDEX_ROOT already being available.  The script doesn't
# actually care about CONFIG_FILE or TREE_NAME, but it's helpful to
# `indexer-logs-analyze.sh`.
$MOZSEARCH_PATH/scripts/compress-outputs.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "compress-outputs.sh"

date

# Check the resulting index for correctness, but there's no webserver so the
# 4th argument needs to be empty.  We now also need the livegrep server to be
# available, so start that first.
$MOZSEARCH_PATH/router/codesearch.py $CONFIG_FILE start $TREE_NAME
date
$MOZSEARCH_PATH/scripts/check-index.sh $CONFIG_FILE $TREE_NAME "filesystem" ""

# And we want to stop it after.  It's possible if we errored above that it will
# still be hanging around, but codesearch.py always stops an existing server
# first, so we're not really concerned about this affecting a re-run of the
# indexing process.
$MOZSEARCH_PATH/router/codesearch.py $CONFIG_FILE stop $TREE_NAME

date
