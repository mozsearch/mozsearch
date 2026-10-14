#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -lt 6 ]]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root> <log-dir> <channel> <dest-email> [WAIT]"
    echo ""
    echo "<channel> and <dest-email> are used when the server continuously fails."
    echo "passing NO_CHANNEL and NO_EMAIL suppresses the behavior."
    echo ""
    echo "WAIT can optionally be passed to wait until the web server is ready."
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

WORKING=$(readlink -f $2)
SERVER_ROOT=$(readlink -f $3)
LOG_DIR=$4
CHANNEL=$5
DEST_EMAIL=$6
WAIT=${7:-}

CONFIG_FILE="$SERVER_ROOT/config.json"
STATUS_FILE="${SERVER_ROOT}/docroot/status.txt"

pkill -x codesearch || true
pkill -x .router-wrapped || true
pkill -x web-server || true
pkill -x pipeline-server || true
# The nix package wraps pipeline-server (see nix/mozsearch/tools.nix), so its
# process is named .pipeline-server-wrapped, which pkill sees as the first 15
# characters.  (Otherwise the old one keeps its port and the new one keeps
# failing to start.)
pkill -x .pipeline-serve || true

sleep 0.1s

nohup \
    $MOZSEARCH_PATH/infrastructure/with-auto-restart.sh \
    $MOZSEARCH_PATH $CHANNEL router $DEST_EMAIL $LOG_DIR/router.err \
    \
    router $CONFIG_FILE $STATUS_FILE \
    > $LOG_DIR/router.log \
    2> $LOG_DIR/router.err \
    < /dev/null &

export RUST_BACKTRACE=1
nohup \
    $MOZSEARCH_PATH/infrastructure/with-auto-restart.sh \
    $MOZSEARCH_PATH $CHANNEL web-server $DEST_EMAIL $LOG_DIR/rust-server.err \
    \
    web-server $CONFIG_FILE $STATUS_FILE \
    > $LOG_DIR/rust-server.log \
    2> $LOG_DIR/rust-server.err \
    < /dev/null &

# Let's try and stop the pipeline-server from causing problems by setting a ulimit
# on virtual memory usage: the worry is a user's unexpectedly expensive dynamic
# query (ex: a huge diagram) growing it until the OOM killer kills other
# processes (the web-server, livegrep, nginx, a try push's build).  It's the
# sizes of the files it mmaps (identifiers, crossref/crossref-extra, and
# jumpref/jumpref-extra), which are file-backed, so they don't need memory of
# their own, plus allowances for other libraries and fundamental mapping, its
# malloc arenas, and its runtime memory usage.
#
# Resulting units are KiB in all cases, which is also what ulimit takes.
MAPPED_FILES_USAGE_K=$(du -c $WORKING/*/identifiers $WORKING/*/crossref* $WORKING/*/jumpref* | cut -f1 | tail -1)
# When first adding the ulimit, our VM size was 13.7G with resident usage of
# 390M.  When writing this on the spare config1 I'm seeing 13.5G VM with 668M
# resident with the MAPPED_FILES_USAGE_K above reporting ~12.7G which gives 800M
STEADY_STATE_ASSUMED_K=$((800 * 1024))
# glibc gives threads their own malloc arenas, each reserving 64 MiB of address
# space, up to 8 per core, and the pipeline-server has a thread per core (and
# more for blocking work), so the arenas alone reserve GBs on many-core
# machines (ex: ~4G with 64 cores, peaking at ~11G as it starts), which the
# rest doesn't allow for: then its index's mmaps fail, and it serves without
# them (ex: a tree's own crossref data, but its linked trees').  So allow two
# arenas per core.
ARENAS_K=$(($(nproc) * 2 * 64 * 1024))
# Allowed growth.  When first adding the ulimit, on 32G machines without swap,
# we allowed 10.3G of VM usage which paired with a 10.7G of resident usage,
# rounded down to 10G since we already grew the steady state above, to bound
# how much memory it could take from the rest.  Machines now have more memory
# (ex: config8's 128G) and swap (see web-serve.sh), so it's a quarter of the
# machine's memory and half of its swap, if that's more: it can grow into swap,
# slowly, rather than failing queries, while most of the memory stays the other
# processes'.
MEM_TOTAL_K=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
SWAP_TOTAL_K=$(awk '/^SwapTotal:/ {print $2}' /proc/meminfo)
ALLOWED_GROWTH_K=$(($MEM_TOTAL_K / 4 + $SWAP_TOTAL_K / 2))
if [[ $ALLOWED_GROWTH_K -lt $((10 * 1024 * 1024)) ]]; then
    ALLOWED_GROWTH_K=$((10 * 1024 * 1024))
fi

# I've also just manually confirmed that this works as expected for config4
# where our sum below ends up at ~48G and the pipeline-server VM ends up at
# ~38G, although that's after doing some brief diagram testing to RES is also
# 1410M, but it works out okay.
PIPELINE_SERVER_VM_LIMIT_K=$(($MAPPED_FILES_USAGE_K + $STEADY_STATE_ASSUMED_K + $ARENAS_K + $ALLOWED_GROWTH_K))
echo "pipeline-server address space limit: $(($PIPELINE_SERVER_VM_LIMIT_K / 1024 / 1024))G (growth $(($ALLOWED_GROWTH_K / 1024 / 1024))G, with ${MEM_TOTAL_K}K of memory and ${SWAP_TOTAL_K}K of swap)"

# ulimit -v units are kilobytes
ulimit -v $PIPELINE_SERVER_VM_LIMIT_K

# Note that we do not currently wait for the pipeline-server and it does not
# write to the STATUS_FILE.
nohup \
    $MOZSEARCH_PATH/infrastructure/with-auto-restart.sh \
    $MOZSEARCH_PATH $CHANNEL pipeline-server $DEST_EMAIL $LOG_DIR/pipeline-server.err \
    \
    pipeline-server $CONFIG_FILE \
    > $LOG_DIR/pipeline-server.log \
    2> $LOG_DIR/pipeline-server.err \
    < /dev/null &

# If WAIT was passed, wait until the servers report they loaded.
if [[ $WAIT = "WAIT" ]]; then
  echo "Waiting for router and web-server to report they have started in ${STATUS_FILE}"
  until [[ $(grep -c loaded ${STATUS_FILE}) -eq 2 ]]; do
    sleep 0.1s
  done
fi
