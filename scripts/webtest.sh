#!/usr/bin/env bash

set -eu
set -o pipefail
set -x

FILTER=${1:-}

stop_geckodriver() {
    PID=$(pgrep geckodriver || true)
    if [ "x${PID}" != "x" ]; then
        echo "Stopping geckodriver: PID=${PID}"
        kill $PID
    fi
}

stop_geckodriver

# Make a FIFO to wait for geckodriver
PIPE=$(mktemp -u)
mkfifo $PIPE
exec {FD}<>$PIPE
rm $PIPE

echo "Starting geckodriver (waiting for it to be ready)"
geckodriver >&$FD &
grep -q 'Listening on' <&$FD

echo "Running tests"
searchfox-tool "webtest ${FILTER}"

stop_geckodriver
