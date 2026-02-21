#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <working-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

export CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)

CONFIG_FILE=$WORKING/config.json

export AWS_ROOT=$MOZSEARCH_PATH/infrastructure/aws

for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
    echo "Performing indexer-upload section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"

    echo "Performing load-vars step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

    if [[ -f $CONFIG_REPO/$TREE_NAME/upload ]]
    then
        echo "Performing upload step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
        $CONFIG_REPO/$TREE_NAME/upload || handle_tree_error "tree upload script"
    fi

    echo "Performed upload section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
done
