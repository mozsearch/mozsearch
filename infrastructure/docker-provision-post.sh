#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

cargo install geckodriver

if ! [ -d mozsearch-firefox ]; then
    curl -L -o mozsearch-firefox.tar.bz2 "https://download.mozilla.org/?product=firefox-latest&os=linux64"
    tar xf mozsearch-firefox.tar.bz2
    mv firefox mozsearch-firefox
fi
