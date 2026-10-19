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

# With "pipeline_trees" in the config, the next tree's early steps (its
# find-repo-files, mkdirs and build: ex: firefox-shared's processing of the
# downloaded Gecko analysis) run while this tree's others do, as a tree's early
# steps only depend on its setup, and most of an indexing's steps leave most of
# the cores idle.  (A tree's early steps' log goes in the main log when they're
# done.)  Not by default, as two trees' steps at once take more memory (ex: a
# developer's container).
PIPELINE_TREES=$(jq -r '.pipeline_trees // false' ${CONFIG_FILE})
EARLY_STEPS=find-repo-files,mkdirs,build
TREES=($(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE}))
EARLY_PID=""
mkdir -p $WORKING/tmp

# Start the early steps of the tree at index $1 in the background.
start_early_steps() {
    local tree=${TREES[$1]}
    (
        . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $tree
        ONLY=$EARLY_STEPS $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $tree
    ) > $WORKING/tmp/early-steps-$tree.log 2>&1 &
    EARLY_PID=$!
}

for TREE_INDEX in "${!TREES[@]}"
do
    TREE_NAME=${TREES[$TREE_INDEX]}
    echo "Performing indexer-run section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"

    echo "Performing load-vars step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

    if [[ $PIPELINE_TREES == "true" ]]; then
        # (This tree's early steps: started with the previous tree's others,
        # or now for the first.)
        if [[ -z $EARLY_PID ]]; then
            start_early_steps $TREE_INDEX
        fi
        EARLY_STATUS=0
        wait $EARLY_PID || EARLY_STATUS=$?
        EARLY_PID=""
        echo "Early steps' log for $TREE_NAME:"
        cat $WORKING/tmp/early-steps-$TREE_NAME.log
        if [[ $EARLY_STATUS != 0 ]]; then
            handle_tree_error "mkindex.sh's early steps"
        fi
        if (( TREE_INDEX + 1 < ${#TREES[@]} )); then
            start_early_steps $((TREE_INDEX + 1))
        fi
        SKIP="${SKIP:+$SKIP,}$EARLY_STEPS" EARLY_STEPS_DONE=1 \
            $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME || handle_tree_error "mkindex.sh"
    else
        $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME || handle_tree_error "mkindex.sh"
    fi
    # If we were given a permanent path, move the index results there and
    # symlink from the old location to the new location.
    if [ -n "$PERMANENT" ]
    then
      mv $INDEX_ROOT ${INDEX_ROOT/$WORKING/$PERMANENT}
      ln -s ${INDEX_ROOT/$WORKING/$PERMANENT} $INDEX_ROOT
    fi

    echo "Performed indexer-run section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
done

# The uploads after setup which indexer-upload.sh started in the background
# (ex: of the history the setup updated) are done before the "*-shared"
# directories move (see there).
$MOZSEARCH_PATH/infrastructure/indexer-upload.sh $CONFIG_REPO $WORKING wait

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
