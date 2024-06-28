#!/usr/bin/env bash

set -e

FILTER=$1

cargo install geckodriver

if ! [ -d mozsearch-firefox ]; then
    curl -L -o mozsearch-firefox.tar.bz2 "https://download.mozilla.org/?product=firefox-latest&os=linux64"
    tar xf mozsearch-firefox.tar.bz2
    mv firefox mozsearch-firefox
fi

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
geckodriver -b /vagrant/mozsearch-firefox/firefox >/dev/null 2>&1 &

echo "Running tests"
./tools/target/release/searchfox-tool "webtest ${FILTER}"

stop_geckodriver
