#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Import a try push into the tree's history as scripts/import-try-push.sh
# does, but together with the other imports waiting to (ex: a spec's pull
# requests imported at once, whose hooks run beside each other; see
# tools/src/try_import.rs): it queues the push in
# HISTORY_ROOT/try-imports/history-queue/, then waits for the history's lock
# (TRY_IMPORT_LOCK, which the caller mustn't hold), and then, unless the import
# which had the lock before it imported its push with its own, it imports its
# push and the others queued since (at most HISTORY_BATCH_MAX, 8 by default,
# those whose hooks are still running), all of their revisions in one history
# run (see scripts/import-try-pushes.sh), leaving the others their results.
# Should that batch fail, each of its pushes is imported alone, so that a
# push's failure is only its own.  Each push's import's log says which batch
# imported it.
#
# HISTORY_BATCH_WAIT (seconds, 0 by default) is how long it waits before
# waiting for the lock, for the other imports to queue theirs (ex: the tests',
# which start at once).
#
# Usage: import-try-push-batched.sh SOURCE_GIT HISTORY_ROOT BRANCH NAME REV MAX_REVISIONS [SYNTAX_ARGS...]
# (as import-try-push.sh's, with PUSH_REVS likewise).

if [ $# -lt 6 ]; then
    echo "Usage: $0 SOURCE_GIT HISTORY_ROOT BRANCH NAME REV MAX_REVISIONS [SYNTAX_ARGS...]"
    exit 1
fi

SOURCE_GIT=$1
HISTORY_ROOT=$2
BRANCH=$3
NAME=$4
REV=$5
MAX_REVISIONS=$6
shift 6

QUEUE="$HISTORY_ROOT/try-imports/history-queue"
mkdir -p "$QUEUE"
# (An entry's file: the PID of the import waiting for it, and its batch line;
# and once a batch imported it, KEY.done: its status and the batch's first
# push.)
KEY=${NAME//\//%}
ENTRY="$QUEUE/$KEY"
rm -f "$ENTRY.done"
printf '%s\n%s %s %s\n' "$$" "$NAME" "$REV" "${PUSH_REVS:-}" > "$ENTRY.tmp"
mv "$ENTRY.tmp" "$ENTRY"

sleep "${HISTORY_BATCH_WAIT:-0}"
echo "Performing import::wait-for-history step for $NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
exec 9>"${TRY_IMPORT_LOCK:-$HISTORY_ROOT/try-imports/history.lock}"
flock 9

if [ -e "$ENTRY.done" ]; then
    read -r STATUS LEADER < "$ENTRY.done"
    rm -f "$ENTRY.done"
    echo "Its history was imported with $LEADER's batch (see its log), with status $STATUS."
    exit "$STATUS"
fi

# Its turn: its push first, then the others queued (oldest first), those whose
# imports are still running.  (And the results of batches whose imports
# aren't waiting for them any more are left out.)
find "$QUEUE" -maxdepth 1 -name "*.done" -mmin +60 -delete
ENTRIES=("$KEY")
while read -r OTHER; do
    OTHER=$(basename "$OTHER")
    [ "$OTHER" != "$KEY" ] || continue
    (( ${#ENTRIES[@]} < ${HISTORY_BATCH_MAX:-8} )) || break
    PID=$(head -1 "$QUEUE/$OTHER" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$QUEUE/$OTHER"
        continue
    fi
    ENTRIES+=("$OTHER")
done < <(find "$QUEUE" -maxdepth 1 -type f ! -name "*.done" ! -name "*.tmp" -printf '%T@ %p\n' | sort -n | cut -d' ' -f2-)

BATCH=$(mktemp)
trap 'rm -f "$BATCH"' EXIT
for E in "${ENTRIES[@]}"; do
    sed -n 2p "$QUEUE/$E"
done > "$BATCH"
echo "Importing the histories of ${#ENTRIES[@]} push(es) at once: $(cut -d' ' -f1 "$BATCH" | tr '\n' ' ')"

# Each entry's status (and the batch's first push, its own, for the others'
# logs).
declare -A STATUSES
if "$MOZSEARCH_PATH/scripts/import-try-pushes.sh" "$SOURCE_GIT" "$HISTORY_ROOT" "$BRANCH" \
        "$MAX_REVISIONS" "$BATCH" "$@"; then
    for E in "${ENTRIES[@]}"; do
        STATUSES[$E]=0
    done
elif (( ${#ENTRIES[@]} == 1 )); then
    STATUSES[$KEY]=1
else
    echo "The batch failed; importing its pushes one at a time."
    for E in "${ENTRIES[@]}"; do
        sed -n 2p "$QUEUE/$E" > "$BATCH"
        if "$MOZSEARCH_PATH/scripts/import-try-pushes.sh" "$SOURCE_GIT" "$HISTORY_ROOT" "$BRANCH" \
                "$MAX_REVISIONS" "$BATCH" "$@"; then
            STATUSES[$E]=0
        else
            STATUSES[$E]=1
        fi
    done
fi

for E in "${ENTRIES[@]}"; do
    rm -f "$QUEUE/$E"
    if [ "$E" != "$KEY" ]; then
        echo "${STATUSES[$E]} $NAME" > "$QUEUE/$E.done"
    fi
done
exit "${STATUSES[$KEY]}"
