#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# scip-python's index of a pyright project (CONFIG, a pyright configuration
# file in ROOT, whose include, exclude and extraPaths are relative to ROOT),
# in chunks indexed side by side (SCIP_PYTHON_CHUNKS processes, by default a
# quarter of the cores): its files, split by directory (deeper where a
# directory has more than a chunk's share of their bytes; see
# scip-python-chunks.py) into chunks with about as many bytes each, each
# chunk's configuration (in ROOT, as .scip-python-chunk-PID-N.json, while
# it's indexed) including its directories, with all of CONFIG's exclusions and
# extraPaths, so that each process resolves imports as one process for the
# whole project would.  Their indexes, concatenated (SCIP indexes are
# concatenations of Index messages), are OUTPUT.  (A file which a chunk
# imports from another chunk has a document in both, the same but for
# pyright's inference's dependence on the order it checks files in, ex: of
# circular imports; scip-indexer keeps the last.)  The ARGS are scip-python's
# (ex: --project-name), and NODE_OPTIONS each process's.
#
# On mozilla-central's Python but the web-platform-tests (5,891 files, as
# firefox-disco's build indexes it), 8 chunks took 2.3 minutes on a 64-core
# machine, rather than 6.0 for one process (9.0 with scip-python's first
# analysis; see nix/scip-python's 0007 patch).
#
# Usage: scip-python-index.sh ROOT CONFIG OUTPUT [ARGS...]

if [ $# -lt 3 ]; then
    echo "Usage: scip-python-index.sh ROOT CONFIG OUTPUT [ARGS...]" 1>&2
    exit 1
fi

ROOT=$(realpath "$1")
CONFIG=$(realpath "$2")
OUTPUT=$(realpath -m "$3")
shift 3
CHUNKS=${SCIP_PYTHON_CHUNKS:-$(( $(nproc) > 4 ? $(nproc) / 4 : 1 ))}

cd "$ROOT"
WORK=$(mktemp -d)
# (Unique, as several can run side by side in ROOT.)
PREFIX=.scip-python-chunk-$$-
trap 'rm -rf "$WORK"; rm -f "$ROOT/$PREFIX"*.json' EXIT
$MOZSEARCH_PATH/scripts/scip-python-chunks.py "$ROOT" "$CONFIG" "$CHUNKS" "$PREFIX"

PIDS=()
for chunk in "$PREFIX"*.json; do
    name=${chunk#.}
    name=${name%.json}
    scip-python index --cwd "$ROOT" --config "$chunk" --show-progress-rate-limit 30 \
        --output "$WORK/$name.scip" "$@" > "$WORK/$name.log" 2>&1 &
    PIDS+=($!)
done
FAILED=
for pid in "${PIDS[@]}"; do
    wait "$pid" || FAILED=1
done
for log in "$WORK"/*.log; do
    echo "== $(basename "$log" .log)"
    if [ -n "$FAILED" ]; then
        cat "$log"
    else
        # (Its steps, not its progress.)
        grep -v '^([0-9:]*)   ' "$log" || true
    fi
done
if [ -n "$FAILED" ]; then
    echo "scip-python failed"
    exit 1
fi
cat "$WORK"/*.scip > "$OUTPUT"
