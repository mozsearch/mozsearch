#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ -n "${CHECK_WARNINGS:-}" ]; then
    exec > >(tee /tmp/indexer-run-log) 2>&1
fi

if [ $# -lt 2 ]
then
    echo "usage: $0 <config-repo-path> <index-path> [permanent-path]"
    exit 1
fi

export MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)
export CONFIG_REPO=$(readlink -f $1)
export WORKING=$(readlink -f $2)
export PERMANENT=${3:+$(readlink -f $3)}

CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
    echo "Performing indexer-run section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"

    echo "Performing load-vars step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

    $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME || handle_tree_error "mkindex.sh"
    # If we were given a permanent path, move the index results there and
    # symlink from the old location to the new location.
    if [ -n "$PERMANENT" ]
    then
      mv $INDEX_ROOT ${INDEX_ROOT/$WORKING/$PERMANENT}
      ln -s ${INDEX_ROOT/$WORKING/$PERMANENT} $INDEX_ROOT
    fi

    echo "Performed indexer-run section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
done

# If we were given a permanent path, move any "*-shared" directories, specifically
# to cover "firefox-shared".
if [ -n "$PERMANENT" ]
then
    # Copy the config file we used but to a different filename since we haven't
    # been doing this and the web-server would clobber "config.json".
    cp -f "$CONFIG_FILE" "$PERMANENT/indexed-config.json"
    # Handle there being no *-shared directories by changing glob expansion to
    # expand to nothing if they don't match (temporarily).
    shopt -s nullglob
    SHARED_SUBDIRS=("$WORKING"/*-shared)
    if (( ${#SHARED_SUBDIRS[@]} )); then
        mv -f "${SHARED_SUBDIRS[@]}" "$PERMANENT"
    fi
    shopt -u nullglob
fi

# Note that we are not moving, but we do expect to still be here:
# - `config.json` - Note that our check-script mechanism actually currently depends
#   on us leaving this here.  And before when we'd mount the index at `~/index`
#   we definitely needed to regenerate this because the paths were all wrong, but
#   now we can potentially leave it as-is.  For now I'm copying it across to a new
#   name above.
# - `tmp/` - We create this and try and point any general TMP use at it, so there
#   could be interesting stuff in here.

if [ -n "${CHECK_WARNINGS:-}" ]; then
    $MOZSEARCH_PATH/infrastructure/aws/send-warning-email.py \
        $MOZSEARCH_PATH/infrastructure/aws/warning-suppression.patterns \
        check-warnings \
        test-error \
        /tmp/indexer-run-log
fi
