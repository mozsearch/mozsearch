#!/usr/bin/env bash

set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A step which runs in the background for the steps after it which wait for
# it, ex: the config repo's wubspecs setup's lookup of its stale anchors
# (update-history.sh), which spec-analyze.sh waits for, so that the indexing,
# which waits for every tree's setup, needn't.
#
#   background-step.sh start PREFIX COMMAND [ARGS...]
#     runs COMMAND (a program, not a shell function) in the background, its
#     output in PREFIX.log and, once it's done, its exit status in
#     PREFIX.status (PREFIX.pending, with its pid, until then);
#   background-step.sh wait PREFIX
#     waits for it, if it was started, shows its log (the first time), and
#     exits with its status (1 if it ended without one, ex: killed).

if [ $# -lt 2 ]; then
    echo "Usage: background-step.sh start PREFIX COMMAND [ARGS...] | wait PREFIX" 1>&2
    exit 1
fi

ACTION=$1
PREFIX=$2
shift 2

case "$ACTION" in
start)
    rm -f "$PREFIX.status" "$PREFIX.log" "$PREFIX.shown"
    # (Written whole, for `wait` to read.)
    echo starting > "$PREFIX.pending.tmp"
    mv "$PREFIX.pending.tmp" "$PREFIX.pending"
    (
        echo "$BASHPID" > "$PREFIX.pending.tmp"
        mv "$PREFIX.pending.tmp" "$PREFIX.pending"
        STATUS=0
        "$@" > "$PREFIX.log" 2>&1 < /dev/null || STATUS=$?
        echo "$STATUS" > "$PREFIX.status.tmp"
        mv "$PREFIX.status.tmp" "$PREFIX.status"
        rm -f "$PREFIX.pending"
    ) > /dev/null 2>&1 < /dev/null &
    echo "Started $PREFIX in the background (pid $!)."
    ;;
wait)
    if [ ! -e "$PREFIX.pending" ] && [ ! -e "$PREFIX.status" ]; then
        exit 0
    fi
    echo "Waiting for $PREFIX : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    while [ ! -e "$PREFIX.status" ]; do
        PID=$(cat "$PREFIX.pending" 2>/dev/null || true)
        # (Its pid, once it's started; the file is gone once it's done, after
        # its status is written.)
        if { [ -z "$PID" ] || { [[ $PID =~ ^[0-9]+$ ]] && ! kill -0 "$PID" 2>/dev/null; }; } &&
            [ ! -e "$PREFIX.status" ]; then
            echo "$PREFIX ended without a status:"
            cat "$PREFIX.log" 2>/dev/null || true
            exit 1
        fi
        sleep 2
    done
    STATUS=$(cat "$PREFIX.status")
    if [ ! -e "$PREFIX.shown" ]; then
        touch "$PREFIX.shown"
        echo "$PREFIX's log (status $STATUS):"
        cat "$PREFIX.log" 2>/dev/null || true
    fi
    exit "$STATUS"
    ;;
*)
    echo "Usage: background-step.sh start PREFIX COMMAND [ARGS...] | wait PREFIX" 1>&2
    exit 1
    ;;
esac
