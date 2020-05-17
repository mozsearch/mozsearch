#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -ne 4 ]]
then
    echo "usage: $0 <config-file> <tree-name> <do-local-check> <server-url>"
    echo ""
    echo "Pass empty strings for do-local-check or server-url to not perform"
    echo "those checks."
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2
CHECK_DISK=$3
CHECK_SERVER_URL=$4

if [[ -x $CONFIG_REPO/$TREE_NAME/check ]]
then
  $CONFIG_REPO/$TREE_NAME/check "$MOZSEARCH_PATH/scripts/check-helper.sh" "$CHECK_DISK" "$CHECK_SERVER_URL"
fi
