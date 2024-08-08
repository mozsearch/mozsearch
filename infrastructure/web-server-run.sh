#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -lt 3 ]]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root> [WAIT]"
    echo ""
    echo "WAIT can optionally be passed to wait until the web server is ready."
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

WORKING=$(readlink -f $2)
CONFIG_FILE=$WORKING/config.json
SERVER_ROOT=$(readlink -f $3)
STATUS_FILE="${SERVER_ROOT}/docroot/status.txt"

pkill codesearch || true
pkill -f router/router.py || true
pkill -f tools/target/release/web-server || true
pkill -f tools/target/release/pipeline-server || true

sleep 0.1s

# activate the venv we created for livegrep so we have access to the grpc
# dependencies.
LIVEGREP_VENV=$HOME/livegrep-venv
source $LIVEGREP_VENV/bin/activate

nohup $MOZSEARCH_PATH/router/router.py $CONFIG_FILE $STATUS_FILE > $SERVER_ROOT/router.log 2> $SERVER_ROOT/router.err < /dev/null &

export RUST_BACKTRACE=1
nohup $MOZSEARCH_PATH/tools/target/release/web-server $CONFIG_FILE $STATUS_FILE > $SERVER_ROOT/rust-server.log 2> $SERVER_ROOT/rust-server.err < /dev/null &

# Let's try and stop the pipeline-server from causing problems by setting a ulimit
# on virtual memory usage.  We use du to figure out the total sizes of all of
# the files we will mmap, specifically: identifiers, crossref/crossref-extra,
# and jumpref/jumpref-extra.  We then add an allowance for other libraries and
# fundamental mapping, plus an allowance for runtime memory usage.
#
# Resulting units are KiB in all cases, which is also what ulimit takes.
MAPPED_FILES_USAGE_K=$(du -c $WORKING/*/identifiers $WORKING/*/crossref* $WORKING/*/jumpref* | cut -f1 | tail -1)
# When first adding the ulimit, our VM size was 13.7G with resident usage of
# 390M.  When writing this on the spare config1 I'm seeing 13.5G VM with 668M
# resident with the MAPPED_FILES_USAGE_K above reporting ~12.7G which gives 800M
STEADY_STATE_ASSUMED_K=$((800 * 1024))
# Allowed growth.  When first adding the ulimit, we allowed 10.3G of VM usage
# which paired with a 10.7G of resident usage.  I'm going to round this down to
# 10G since we already grew the steady state above.
ALLOWED_GROWTH_K=$((10 * 1024 * 1024))

# I've also just manually confirmed that this works as expected for config4
# where our sum below ends up at ~48G and the pipeline-server VM ends up at
# ~38G, although that's after doing some brief diagram testing to RES is also
# 1410M, but it works out okay.
PIPELINE_SERVER_VM_LIMIT_K=$(($MAPPED_FILES_USAGE_K + $STEADY_STATE_ASSUMED_K + $ALLOWED_GROWTH_K))

# ulimit -v units are kilobytes
ulimit -v $PIPELINE_SERVER_VM_LIMIT_K

# Note that we do not currently wait for the pipeline-server and it does not
# write to the STATUS_FILE.
nohup $MOZSEARCH_PATH/tools/target/release/pipeline-server $CONFIG_FILE > $SERVER_ROOT/pipeline-server.log 2> $SERVER_ROOT/pipeline-server.err < /dev/null &

# If WAIT was passed, wait until the servers report they loaded.
if [[ ${4:-} = "WAIT" ]]; then
  until [[ $(grep -c loaded ${STATUS_FILE}) -eq 2 ]]; do
    sleep 0.1s
  done
fi
