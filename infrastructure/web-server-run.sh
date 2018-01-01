#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

WORKING=$(readlink -f $2)
CONFIG_FILE=$WORKING/config.json
SERVER_ROOT=$(readlink -f $3)

pkill codesearch || true
pkill -f router/router.py || true
pkill -f tools/target/release/web-server || true

sleep 1

nohup python $MOZSEARCH_PATH/router/router.py $CONFIG_FILE > $SERVER_ROOT/router.log 2> $SERVER_ROOT/router.err < /dev/null &

export RUST_BACKTRACE=1
nohup $MOZSEARCH_PATH/tools/target/release/web-server $CONFIG_FILE > $SERVER_ROOT/rust-server.log 2> $SERVER_ROOT/rust-server.err < /dev/null &
