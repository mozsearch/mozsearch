#!/bin/bash

set -e
set -x

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <base-path> <temp-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

CONFIG_REPO=$(readlink -f $1)
BASE=$(readlink -f $2)
TEMP=$(readlink -f $3)

CONFIG_FILE=$TEMP/config.json

export MOZSEARCH_PATH
export BASE
export TEMP
envsubst < $CONFIG_REPO/config.json > $CONFIG_FILE
