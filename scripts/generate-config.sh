#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 4 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <index-path> <output-path>"
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"
WORKING=$(readlink -f $3)
OUTPUT_DIR=$(readlink -f $4)

CONFIG_FILE=$OUTPUT_DIR/config.json

$MOZSEARCH_PATH/scripts/composite-config.py \
    $CONFIG_REPO/$CONFIG_INPUT \
    $CONFIG_FILE \
    $MOZSEARCH_PATH $CONFIG_REPO $WORKING ${MOZSEARCH_SOURCE_PATH:-""}
