#!/usr/bin/env bash

set -eu
set -o pipefail
set -x

FILTER=${1:-}

# How many browsers to run the tests in, at once (see the tools'
# `cmd_webtest`).  More aren't faster, even with many cores: the trydelta-push
# group's tests, one after another, take about all of the time, and their
# imports slow down beside more of the others' (in the development container's
# 64 cores, 8 browsers took 110 s, rather than 4's 100).  (8 browsers' threads
# also need more tasks than podman's default limit; see run-docker.sh's
# --pids-limit.)
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
    # (Tests' steps in the browser's own context need its system access; see
    # cmd_webtest.rs's `run_chrome`.)
    geckodriver --port $PORT --allow-system-access > "$LOGS/geckodriver-$i.log" 2>&1 &
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
