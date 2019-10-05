#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <working-path>"
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"
WORKING=$(readlink -f $3)

CONFIG_FILE=$WORKING/config.json

export MOZSEARCH_PATH
export WORKING
envsubst < $CONFIG_REPO/$CONFIG_INPUT > $CONFIG_FILE
