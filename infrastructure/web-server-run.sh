#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -lt 3 ]]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root> [WAIT]"
    echo ""
    echo "WAIT can optionally be passed to wait until the web server is ready."
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

WORKING=$(readlink -f $2)
CONFIG_FILE=$WORKING/config.json
SERVER_ROOT=$(readlink -f $3)
STATUS_FILE="${SERVER_ROOT}/docroot/status.txt"

pkill codesearch || true
pkill -f router/router.py || true
pkill -f tools/target/release/web-server || true
pkill -f tools/target/release/pipeline-server || true

sleep 0.1s

nohup $MOZSEARCH_PATH/router/router.py $CONFIG_FILE $STATUS_FILE > $SERVER_ROOT/router.log 2> $SERVER_ROOT/router.err < /dev/null &

export RUST_BACKTRACE=1
nohup $MOZSEARCH_PATH/tools/target/release/web-server $CONFIG_FILE $STATUS_FILE > $SERVER_ROOT/rust-server.log 2> $SERVER_ROOT/rust-server.err < /dev/null &

# Note that we do not currently wait for the pipeline-server and it does not
# write to the STATUS_FILE.
nohup $MOZSEARCH_PATH/tools/target/release/pipeline-server $CONFIG_FILE > $SERVER_ROOT/pipeline-server.log 2> $SERVER_ROOT/pipeline-server.err < /dev/null &

# If WAIT was passed, wait until the servers report they loaded.
if [[ ${4:-} = "WAIT" ]]; then
  until [[ $(grep -c loaded ${STATUS_FILE}) -eq 2 ]]; do
    sleep 0.1s
  done
fi
