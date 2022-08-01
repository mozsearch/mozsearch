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
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

    # If the setup script failed, indexer-setup.sh will have used the
    # `inhibit_upload` function exported by `load-vars.sh` to create an
    # `INHIBIT_UPLOAD` marker at the root of the tree.
    if [ -f $CONFIG_REPO/$TREE_NAME/upload && ! -f $INDEX_ROOT/INHIBIT_UPLOAD ]
    then
        $CONFIG_REPO/$TREE_NAME/upload || handle_tree_error "tree upload script"
    fi
done
