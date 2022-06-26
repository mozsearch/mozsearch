#!/usr/bin/env bash

# This script is intended to be run inside the vagrant VM to produce lambda
# job zip files for our daily-run searchfox indexing jobs.

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

AWSDIR=$(dirname $0)

# create lambda-release1.zip
$AWSDIR/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config1.json \
  master \
  release1

# create lambda-release2.zip
$AWSDIR/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config2.json \
  master \
  release2

# create lambda-release3.zip
$AWSDIR/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config3.json \
  master \
  release3

# create lambda-release4.zip
$AWSDIR/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config4.json \
  master \
  release4

# create lambda-release5.zip
$AWSDIR/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config5.json \
  master \
  release5
