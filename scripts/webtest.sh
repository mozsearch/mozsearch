#!/usr/bin/env bash

set -e

FILTER=$1

stop_geckodriver() {
    PID=$(pgrep geckodriver)
    if [ "x${PID}" != "x" ]; then
        echo "Stopping geckodriver: PID=${PID}"
        kill $PID
    fi
}

set +e

stop_geckodriver

echo "Starting geckodriver"
geckodriver >/dev/null 2>&1 &

echo "Running tests"
searchfox-tool "webtest ${FILTER}"

stop_geckodriver
