#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# We now need the aws-cli for the firefox-* branches.  On Ubuntu the supported
# options are either using the "snap" package or the command line installer.
# We're opting for the command-line installer because we're not crazy about
# self-updating things in general and snaps in particular, although this is a
# weakly held opinion as the last time updating burned us was with Python debug
# symbols, not a problem we've had in a while.
curl "https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip" -o "awscliv2.zip"
unzip awscliv2.zip
sudo ./aws/install

cargo install geckodriver

if ! [ -d mozsearch-firefox ]; then
    curl -L -o mozsearch-firefox.tar.bz2 "https://download.mozilla.org/?product=firefox-latest&os=linux64"
    tar xf mozsearch-firefox.tar.bz2
    mv firefox mozsearch-firefox
fi
