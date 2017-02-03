#!/bin/bash

set -e
set -x

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <working-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)

CONFIG_FILE=$WORKING/config.json

export MOZSEARCH_PATH
export WORKING
envsubst < $CONFIG_REPO/config.json > $CONFIG_FILE
