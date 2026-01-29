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

export PYTHONPATH="$MOZSEARCH_PATH/scripts${PYTHONPATH:+:${PYTHONPATH}}"

# activate the venv we created for livegrep
# TODO: remove after next provisioning
LIVEGREP_VENV="$HOME/livegrep-venv"
PATH="$LIVEGREP_VENV/bin:$PATH"

# TODO: remove after next provisioning
export MOZSEARCH_WASM_DIR=${MOZSEARCH_WASM_DIR:-"$MOZSEARCH_PATH/scripts/web-analyze/wasm-css-analyzer/out"}
export MOZSEARCH_CLANG_PLUGIN_DIR=${MOZSEARCH_CLANG_PLUGIN_DIR:-"$MOZSEARCH_PATH/clang-plugin"}

# This was previously "full" but "1" is much more readable.  Obviously change
# this back if we end up missing things.
#
# Also note that if "parallel" is used, it's necessary to add an arg pair of
# "--env RUST_BACKTRACE" unless using "env_parallel".
export RUST_BACKTRACE=1

# Each step below has an associated name, and can be skipped with the following
# environment variables:
#   SKIP
#     A comma-separated list of names.
#     Steps included in the list are skipped.
#   SKIP_UNTIL
#     Steps before the step with this name are skipped.
#
# The following steps should be handled carefully when re-running with skipping
# some steps:
#   mkdirs
#     This step removes all intermedia files/directories.
#     If you're going to skip some steps that generates analysis etc,
#     this step should also be skipped.
#   compress-outputs
#     This step compresses the output and the analysis files.
#     If you're going to resume from earlier steps that depends on the
#     raw analysis file, this step should be skipped in the initial run.
#     If SKIP_UNTIL is set, those files are automatically uncompressed,
#     But skipping the compress-outputs step in the initial run will
#     reduce the turn around time.
#
# Common use cases:
#   Modify js-analyze.js and check the result:
#     SKIP=compress-outputs make review-test-repo
#     and then
#     SKIP_UNTIL=js-analyze SKIP=compress-outputs make review-test-repo
#
#   Modify crossref handling and check the output:
#     SKIP=compress-outputs make review-test-repo
#     and then
#     SKIP_UNTIL=crossref SKIP=compress-outputs make review-test-repo
#
#   Modify anything that does not affect build:
#     SKIP=compress-outputs make review-test-repo
#     and then
#     SKIP=mkdirs,build,compress-outputs make review-test-repo
should_perform() {
    if [[ ${SKIP:-} != "" ]]; then
        if echo ",$SKIP," | grep ",$1," > /dev/null; then
            echo "Skipping $1 step"
            return 1
        fi
    fi

    if [[ ${SKIP_UNTIL:-} == "$1" ]]; then
        SKIP_UNTIL=
    fi

    if [[ ${SKIP_UNTIL:-} != "" ]]; then
        echo "Skipping $1 step"
        return 1
    fi

    echo "Performing $1 step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    return 0
}

if should_perform "find-repo-files"; then
    $MOZSEARCH_PATH/scripts/find-repo-files.py $CONFIG_REPO $CONFIG_FILE $TREE_NAME
fi


if should_perform "mkdirs"; then
    # NOTE: This step removes all intermediate files/directories from the
    #       previous run.
    $MOZSEARCH_PATH/scripts/mkdirs.sh
else
    SKIPPED_MKDIRS=1
fi

if [[ ${SKIPPED_MKDIRS:-} -eq 1 ]]; then
    # NOTE: Given that mkdirs is skipped, the intermediate files/directories
    #       are reused from the previous run.
    #       undo the compress-outputs step, just in case the previous run
    #       didn't skip the compress-outputs step.
    $MOZSEARCH_PATH/scripts/uncompress-outputs.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "uncompress-outputs.sh"
fi

if should_perform "build"; then
    $MOZSEARCH_PATH/scripts/build.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME
fi

export RUST_LOG=info

if should_perform "scip-analyze"; then
    # Transform any .scip files the config `scip_subtrees` setting tells us
    # about. This does not generate any .scip files; instead it is assumed that
    # they would have been generated in the config's `build` script.  It's also
    # assumed that for complicated situations like firefox-main where
    # merge-analyses may be required, that the scripts will handle calling
    # `scip-analyze.sh` or `scip-indexer` directly and will not list them in
    # `scip_subtrees`.
    $MOZSEARCH_PATH/scripts/scip-analyze.sh \
        "$CONFIG_FILE" \
        "$TREE_NAME" || handle_tree_error "scip-analyze.sh"
fi

if should_perform "find-objdir-files"; then
    $MOZSEARCH_PATH/scripts/find-objdir-files.sh
fi

if should_perform "objdir-mkdirs"; then
    $MOZSEARCH_PATH/scripts/objdir-mkdirs.sh
fi

URL_MAP_PATH=$INDEX_ROOT/aliases/url-map.json
DOC_TREES_MAP=$INDEX_ROOT/doc-trees.json

if should_perform "process-chrome-map"; then
    $MOZSEARCH_PATH/scripts/process-chrome-map.py $GIT_ROOT $URL_MAP_PATH $INDEX_ROOT/*.chrome-map.json || handle_tree_error "process-chrome-map.py"
fi

if should_perform "js-analyze"; then
    $MOZSEARCH_PATH/scripts/js-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "js-analyze.sh"
fi

if should_perform "html-analyze"; then
    $MOZSEARCH_PATH/scripts/html-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "html-analyze.sh"
fi

if should_perform "css-analyze"; then
    $MOZSEARCH_PATH/scripts/css-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "css-analyze.sh"
fi

if should_perform "idl-analyze"; then
    $MOZSEARCH_PATH/scripts/idl-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "idl-analyze.sh"
fi

if should_perform "staticprefs-analyze"; then
    $MOZSEARCH_PATH/scripts/staticprefs-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "idl-analyze.sh"
fi

if should_perform "ipdl-analyze"; then
    $MOZSEARCH_PATH/scripts/ipdl-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "ipdl-analyze.sh"
fi

if should_perform "toml-analyze"; then
    $MOZSEARCH_PATH/scripts/toml-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "toml-analyze.sh"
fi

if should_perform "yaml-analyze"; then
    $MOZSEARCH_PATH/scripts/yaml-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "yaml-analyze.sh"
fi

if should_perform "mozbuild-analyze"; then
    $MOZSEARCH_PATH/scripts/mozbuild-analyze.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "mozbuild-analyze.sh"
fi

ANALYSIS_FILES_PATH=$INDEX_ROOT/all-analysis-files

if should_perform "generate-analsysis-files-list"; then
    $MOZSEARCH_PATH/scripts/generate-analsysis-files-list.sh $ANALYSIS_FILES_PATH || handle_tree_error "generate-analsysis-files-list.sh"
fi

if should_perform "replace-aliases"; then
    $MOZSEARCH_PATH/scripts/replace-aliases.sh $ANALYSIS_FILES_PATH || handle_tree_error "replace-aliases.sh"
fi

if should_perform "annotate-gc"; then
    $MOZSEARCH_PATH/scripts/annotate-gc.sh $ANALYSIS_FILES_PATH || handle_tree_error "annotate-gc.sh"
fi

if should_perform "crossref"; then
    # crossref failures always need to be fatal because their outputs are required.
    $MOZSEARCH_PATH/scripts/crossref.sh $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH
fi

if should_perform "output"; then
    $MOZSEARCH_PATH/scripts/output.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME $URL_MAP_PATH $DOC_TREES_MAP || handle_tree_error "output.sh"
fi

if should_perform "build-codesearch"; then
    $MOZSEARCH_PATH/scripts/build-codesearch.py $CONFIG_FILE $TREE_NAME || handle_tree_error "build-codesearch.py"
fi

if should_perform "compress-outputs"; then
    # This depends on INDEX_ROOT already being available.  The script doesn't
    # actually care about CONFIG_FILE or TREE_NAME, but it's helpful to
    # `indexer-logs-analyze.sh`.
    $MOZSEARCH_PATH/scripts/compress-outputs.sh $CONFIG_FILE $TREE_NAME || handle_tree_error "compress-outputs.sh"
fi

if should_perform "codesearch-start"; then
    # Check the resulting index for correctness, but there's no webserver so the
    # 4th argument needs to be empty.  We now also need the livegrep server to be
    # available, so start that first.
    $MOZSEARCH_PATH/router/codesearch.py $CONFIG_FILE start $TREE_NAME
fi

if should_perform "check-index"; then
    $MOZSEARCH_PATH/scripts/check-index.sh $CONFIG_FILE $TREE_NAME "filesystem" ""
fi

if should_perform "codesearch-stop"; then
    # And we want to stop it after.  It's possible if we errored above that it will
    # still be hanging around, but codesearch.py always stops an existing server
    # first, so we're not really concerned about this affecting a re-run of the
    # indexing process.
    $MOZSEARCH_PATH/router/codesearch.py $CONFIG_FILE stop $TREE_NAME
fi
