#!/usr/bin/env bash

# Start the pipeline-server, under its own `with-auto-restart.sh` supervisor,
# with its memory limited so that a user's unexpectedly expensive dynamic
# query (ex: a huge diagram, a big try push's slicing, a burst of the data
# API's analysis files) can't grow it until the machine is unresponsive or
# the OOM killer kills other processes (the web-server, livegrep, nginx, a
# try push's build).  web-server-run.sh runs this; it can also be run alone
# to restart just the pipeline-server (stop the old one first; see below).
#
# Where systemd runs (the web servers), the supervisor runs in a transient
# service, `searchfox-pipeline-server`, whose cgroup limits the memory the
# server uses, rather than its address space: above `MemoryHigh` (3/8 of the
# machine's memory) the kernel throttles it and reclaims its memory first
# (its mmapped index files' and git packs' clean pages are dropped, and read
# again when needed; its own memory goes to swap, up to `MemorySwapMax`, half
# of the swap), and at `MemoryMax` (half of the memory) the OOM killer kills
# it alone (`OOMPolicy=continue`, so that its supervisor restarts it, rather
# than systemd stopping the service).  The address space limit this replaced
# counted its mappings, which aren't memory: config8's index files (60 GB),
# git packs (22 GB) and malloc arenas' reservations (17 GB) left it 5.6 GB of
# its 108 GB (2026-10-19).  `systemctl status searchfox-pipeline-server`
# shows it; `sudo systemctl stop searchfox-pipeline-server` stops it.
#
# Elsewhere (ex: the container's suite), it gets an address space limit
# (`ulimit -v`), as before.

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -ne 6 ]]
then
    echo "usage: $0 <index-path> <server-root> <log-dir> <channel> <dest-email> <config-file>"
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

WORKING=$(readlink -f $1)
SERVER_ROOT=$(readlink -f $2)
LOG_DIR=$3
CHANNEL=$4
DEST_EMAIL=$5
CONFIG_FILE=$6

UNIT=searchfox-pipeline-server

MEM_TOTAL_K=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
SWAP_TOTAL_K=$(awk '/^SwapTotal:/ {print $2}' /proc/meminfo)

SUPERVISOR=(
    $MOZSEARCH_PATH/infrastructure/with-auto-restart.sh
    $MOZSEARCH_PATH $CHANNEL pipeline-server $DEST_EMAIL $LOG_DIR/pipeline-server.err
    pipeline-server $CONFIG_FILE
)

if [[ -d /run/systemd/system ]] && sudo -n true 2>/dev/null; then
    MEMORY_HIGH_K=$(($MEM_TOTAL_K * 3 / 8))
    MEMORY_MAX_K=$(($MEM_TOTAL_K / 2))
    SWAP_MAX_K=$(($SWAP_TOTAL_K / 2))
    echo "pipeline-server memory: high $(($MEMORY_HIGH_K / 1024 / 1024))G, max $(($MEMORY_MAX_K / 1024 / 1024))G, swap $(($SWAP_MAX_K / 1024 / 1024))G"

    # (The previous one's, if it's still there.)
    sudo systemctl stop $UNIT 2>/dev/null || true
    sudo systemctl reset-failed $UNIT 2>/dev/null || true

    # (Its logs are this user's, as they would be without systemd, which
    # creates the files it opens as root: so they're made here, and it
    # appends to them.)
    : > $LOG_DIR/pipeline-server.log
    : > $LOG_DIR/pipeline-server.err

    # (Its environment is this one's, as the supervisor's would be without
    # systemd: sudo's isn't.  Not traced, as it's all of the variables.)
    set +x
    ENV_ARGS=()
    while IFS= read -r NAME; do
        if [[ $NAME =~ ^[A-Za-z_][A-Za-z0-9_]*$ && $NAME != SUDO_* ]]; then
            ENV_ARGS+=("--setenv=$NAME=${!NAME}")
        fi
    done < <(compgen -e)

    sudo systemd-run --unit=$UNIT --collect --quiet \
        --uid="$(id -u)" --gid="$(id -g)" \
        --working-directory="$PWD" \
        "${ENV_ARGS[@]}" \
        -p MemoryHigh=${MEMORY_HIGH_K}K \
        -p MemoryMax=${MEMORY_MAX_K}K \
        -p MemorySwapMax=${SWAP_MAX_K}K \
        -p OOMPolicy=continue \
        -p StandardInput=null \
        -p StandardOutput=append:$LOG_DIR/pipeline-server.log \
        -p StandardError=append:$LOG_DIR/pipeline-server.err \
        "${SUPERVISOR[@]}"
    set -x
    systemctl status $UNIT --no-pager --lines=0 || true
    exit 0
fi

# The address space limit: the sizes of the files it mmaps (identifiers,
# crossref/crossref-extra, and jumpref/jumpref-extra), which are file-backed,
# so they don't need memory of their own, plus allowances for other libraries
# and fundamental mapping, its malloc arenas, and its runtime memory usage.
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
# how much memory it could take from the rest.  It's a quarter of the
# machine's memory and half of its swap, if that's more: it can grow into
# swap, slowly, rather than failing queries, while most of the memory stays
# the other processes'.
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
nohup "${SUPERVISOR[@]}" \
    > $LOG_DIR/pipeline-server.log \
    2> $LOG_DIR/pipeline-server.err \
    < /dev/null &
