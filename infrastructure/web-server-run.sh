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

nohup $MOZSEARCH_PATH/router/router.py $CONFIG_FILE $STATUS_FILE > $SERVER_ROOT/router.log 2> $SERVER_ROOT/router.err < /dev/null &

export RUST_BACKTRACE=1
nohup $MOZSEARCH_PATH/tools/target/release/web-server $CONFIG_FILE $STATUS_FILE > $SERVER_ROOT/rust-server.log 2> $SERVER_ROOT/rust-server.err < /dev/null &

# let's try and stop the pipeline-server from causing problems by setting a ulimit
# on virtual memory usage.  Currently, the worst-case scenario is mozilla-central
# where it starts with 13.7G of VM usage where we know we have 8.9G of memory
# mapped files.  At that point the resident memory usage is 390M.  If we trigger
# a worst-case scenario graph traversal (before fixes), we see 24.0G virt, 11.1G
# resident.  We're setting our virt limit to 24G accordingly since on a t3.xlarge
# that only has 16G of RAM, that leaves enough for the rest not to fall over.
#
# TODO: automate updating/calculating this number somewhat.  In particular, the
# indexer check mechanism will stand up the web-server, which makes it possible
# to inspect the pipeline-server's VM use at that time and then write it to a
# file and then do some math on it.  I'm not doing that right now because it's
# more work and more likely to have problems that defeat the point.  Also,
# there are some upsides to having this be an absolute value that's 3/4 of the
# expected system memory for t3.2xlarge.
#
# ulimit -v units are kilobytes, so we do 24 * 1024 * 1024.
ulimit -v 25165824

# Note that we do not currently wait for the pipeline-server and it does not
# write to the STATUS_FILE.
nohup $MOZSEARCH_PATH/tools/target/release/pipeline-server $CONFIG_FILE > $SERVER_ROOT/pipeline-server.log 2> $SERVER_ROOT/pipeline-server.err < /dev/null &

# If WAIT was passed, wait until the servers report they loaded.
if [[ ${4:-} = "WAIT" ]]; then
  until [[ $(grep -c loaded ${STATUS_FILE}) -eq 2 ]]; do
    sleep 0.1s
  done
fi
