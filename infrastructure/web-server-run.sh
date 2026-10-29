#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -lt 6 ]]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root> <log-dir> <channel> <dest-email> [WAIT]"
    echo ""
    echo "<channel> and <dest-email> are used when the server continuously fails."
    echo "passing NO_CHANNEL and NO_EMAIL suppresses the behavior."
    echo ""
    echo "WAIT can optionally be passed to wait until the web server is ready."
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

WORKING=$(readlink -f $2)
SERVER_ROOT=$(readlink -f $3)
LOG_DIR=$4
CHANNEL=$5
DEST_EMAIL=$6
WAIT=${7:-}

CONFIG_FILE="$SERVER_ROOT/config.json"
STATUS_FILE="${SERVER_ROOT}/docroot/status.txt"

pkill -x codesearch || true
pkill -x .router-wrapped || true
pkill -x web-server || true
pkill -x pipeline-server || true
# The nix package wraps pipeline-server (see nix/mozsearch/tools.nix), so its
# process is named .pipeline-server-wrapped, which pkill sees as the first 15
# characters.  (Otherwise the old one keeps its port and the new one keeps
# failing to start.)
pkill -x .pipeline-serve || true

sleep 0.1s

nohup \
    $MOZSEARCH_PATH/infrastructure/with-auto-restart.sh \
    $MOZSEARCH_PATH $CHANNEL router $DEST_EMAIL $LOG_DIR/router.err \
    \
    router $CONFIG_FILE $STATUS_FILE \
    > $LOG_DIR/router.log \
    2> $LOG_DIR/router.err \
    < /dev/null &

export RUST_BACKTRACE=1
nohup \
    $MOZSEARCH_PATH/infrastructure/with-auto-restart.sh \
    $MOZSEARCH_PATH $CHANNEL web-server $DEST_EMAIL $LOG_DIR/rust-server.err \
    \
    web-server $CONFIG_FILE $STATUS_FILE \
    > $LOG_DIR/rust-server.log \
    2> $LOG_DIR/rust-server.err \
    < /dev/null &

# The pipeline-server, with its memory limited (see pipeline-server-run.sh).
$MOZSEARCH_PATH/infrastructure/pipeline-server-run.sh \
    $WORKING $SERVER_ROOT $LOG_DIR $CHANNEL $DEST_EMAIL $CONFIG_FILE

# If WAIT was passed, wait until the servers report they loaded, and until the
# pipeline-server, which doesn't (see pipeline-server-run.sh), takes
# connections, which it does once it has loaded: the checks' requests for the
# data API (`/{tree}/api/`, which nginx sends to it) got 502s when they came
# before (ex: big_cpp.cpp's analysis, as the webtest suite started).
if [[ $WAIT = "WAIT" ]]; then
  echo "Waiting for router and web-server to report they have started in ${STATUS_FILE}"
  until [[ $(grep -c loaded ${STATUS_FILE}) -eq 2 ]]; do
    sleep 0.1s
  done
  echo "Waiting for the pipeline-server to take connections"
  WAITED=0
  until (exec 3<> /dev/tcp/127.0.0.1/8002) 2> /dev/null; do
    if (( WAITED >= 3000 )); then
      echo "WARNING: The pipeline-server isn't taking connections after 5 minutes; see ${LOG_DIR}/pipeline-server.err"
      break
    fi
    sleep 0.1s
    WAITED=$((WAITED + 1))
  done
fi
