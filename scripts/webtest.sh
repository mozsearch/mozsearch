#!/usr/bin/env bash

set -eu
set -o pipefail
set -x

FILTER=${1:-}

cargo install geckodriver

if ! [ -d mozsearch-firefox ]; then
    curl -L -o mozsearch-firefox.tar.bz2 "https://download.mozilla.org/?product=firefox-latest&os=linux64"
    tar xf mozsearch-firefox.tar.bz2
    mv firefox mozsearch-firefox
fi

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
geckodriver -b "$(pwd)/mozsearch-firefox/firefox" >&$FD &
grep -q 'Listening on' <&$FD

echo "Running tests"
./tools/target/release/searchfox-tool "webtest ${FILTER}"

stop_geckodriver
