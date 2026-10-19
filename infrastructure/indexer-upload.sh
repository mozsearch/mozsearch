#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 2 ] && [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <working-path> [script-name]"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

export CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)

CONFIG_FILE=$WORKING/config.json

# The trees' scripts to run: "upload" after setup (ex: a history setup
# updated), or "upload-after-index" after indexing (ex: a cache indexing
# updated, like spec-analyze's of stale anchors, for the next indexing).
#
# The "upload" scripts run in the background, while indexing goes on (which
# only reads what they upload; ex: firefox-disco's history's took 9 minutes),
# each with its own log; "wait" (see indexer-run.sh, before it ends) waits for
# them, puts their logs in the main log, and warns about those which failed,
# rather than failing the indexing, whose index is fine (and the next
# indexing's setup starts from what was uploaded before).
SCRIPT=${3:-upload}
UPLOADS=$WORKING/tmp/uploads

export AWS_ROOT=$MOZSEARCH_PATH/infrastructure/aws

if [[ $SCRIPT == "wait" ]]; then
    for STARTED in $UPLOADS/*.started
    do
        [[ -f $STARTED ]] || continue
        TREE_NAME=$(basename $STARTED .started)
        while [[ ! -f $UPLOADS/$TREE_NAME.status ]]; do
            sleep 5
        done
        echo "Upload's log for $TREE_NAME:"
        cat $UPLOADS/$TREE_NAME.log
        STATUS=$(cat $UPLOADS/$TREE_NAME.status)
        if [[ $STATUS != 0 ]]; then
            echo "warning: Tree '$TREE_NAME' error: tree upload script failed ($STATUS)"
        fi
        echo "Performed waiting for upload for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    done
    exit 0
fi

for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
    echo "Performing indexer-upload section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"

    echo "Performing load-vars step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

    if [[ -f $CONFIG_REPO/$TREE_NAME/$SCRIPT && $SCRIPT == "upload" ]]
    then
        echo "Starting $SCRIPT step in the background for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
        mkdir -p $UPLOADS
        rm -f $UPLOADS/$TREE_NAME.status
        touch $UPLOADS/$TREE_NAME.started
        (
            STATUS=0
            $CONFIG_REPO/$TREE_NAME/$SCRIPT > $UPLOADS/$TREE_NAME.log 2>&1 || STATUS=$?
            echo $STATUS > $UPLOADS/$TREE_NAME.status.tmp
            mv $UPLOADS/$TREE_NAME.status.tmp $UPLOADS/$TREE_NAME.status
        ) &
    elif [[ -f $CONFIG_REPO/$TREE_NAME/$SCRIPT ]]
    then
        echo "Performing $SCRIPT step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
        $CONFIG_REPO/$TREE_NAME/$SCRIPT || handle_tree_error "tree $SCRIPT script"
    fi

    echo "Performed upload section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
done
