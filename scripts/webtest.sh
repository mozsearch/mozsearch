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

echo "Starting geckodriver"
geckodriver >/dev/null 2>&1 &

sleep 10

echo "Running tests"
searchfox-tool "webtest ${FILTER}"

stop_geckodriver
