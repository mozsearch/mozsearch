#!/usr/bin/env bash

set -eu
set -o pipefail
set -x

FILTER=${1:-}

# How many browsers to run the tests in, at once (see the tools'
# `cmd_webtest`).
BROWSERS=${WEBTEST_BROWSERS:-4}

stop_geckodriver() {
    PIDS=$(pgrep -x geckodriver || true)
    if [ "x${PIDS}" != "x" ]; then
        echo "Stopping geckodriver: PIDs=${PIDS}"
        kill $PIDS
    fi
}

stop_geckodriver

echo "Starting geckodriver x${BROWSERS} (waiting for them to be ready)"
# (Their output, Firefox's console's included, for after.)
LOGS=/tmp/webtest-geckodriver
rm -rf "$LOGS"
mkdir -p "$LOGS"
WEBDRIVERS=""
for i in $(seq 0 $((BROWSERS - 1))); do
    PORT=$((4444 + i))
    geckodriver --port $PORT > "$LOGS/geckodriver-$i.log" 2>&1 &
    WEBDRIVERS="${WEBDRIVERS:+$WEBDRIVERS,}http://localhost:$PORT"
done
for i in $(seq 0 $((BROWSERS - 1))); do
    until grep -q 'Listening on' "$LOGS/geckodriver-$i.log"; do
        sleep 0.1
    done
done

echo "Running tests"
STATUS=0
searchfox-tool "webtest ${FILTER} --webdrivers ${WEBDRIVERS}" || STATUS=$?

stop_geckodriver
exit $STATUS
