#!/bin/bash

set -e
set -x

if [ $# != 1 ]
then
    echo "usage: $0 <temp-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

TEMP_PATH=$(readlink -f $1)
CONFIG_FILE=$TEMP_PATH/config.json

nohup python $MOZSEARCH_PATH/router/router.py $CONFIG_FILE > router.log 2> router.err < /dev/null &

nohup $MOZSEARCH_PATH/tools/target/release/web-server $CONFIG_FILE > rust-server.log 2> rust-server.err < /dev/null &
