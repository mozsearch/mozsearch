#!/usr/bin/env bash

# Print how many jobs of about $1 MiB of memory each a step can run at once:
# as many as fit in the memory and the cores.  In a container (ex: a
# developer's searchfox container, which shares the machine with everything
# else they run), half of the memory and the cores less 2 (for their
# interactive sessions); otherwise (ex: an indexer, which is ours alone), three
# quarters of the memory (of the cgroup's limit, if it's lower) and all of the
# cores.  `PARALLEL_JOBS` overrides it.

set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 1 ]; then
    echo "Usage: parallel-jobs.sh <MiB per job>" >&2
    exit 1
fi

if [ -n "${PARALLEL_JOBS:-}" ]; then
    echo "$PARALLEL_JOBS"
    exit 0
fi

PER_JOB_MIB=$1
CORES=$(nproc)

MEM_KIB=$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)
for LIMIT_FILE in /sys/fs/cgroup/memory.max /sys/fs/cgroup/memory/memory.limit_in_bytes; do
    if [ -r "$LIMIT_FILE" ]; then
        LIMIT=$(cat "$LIMIT_FILE")
        if [[ $LIMIT =~ ^[0-9]+$ ]] && (( LIMIT / 1024 < MEM_KIB )); then
            MEM_KIB=$(( LIMIT / 1024 ))
        fi
    fi
done

if [ -f /.dockerenv ] || [ -f /run/.containerenv ]; then
    SHARE=50
    CPU_JOBS=$(( CORES > 3 ? CORES - 2 : 1 ))
else
    SHARE=75
    CPU_JOBS=$CORES
fi
MEM_JOBS=$(( MEM_KIB * SHARE / 100 / 1024 / PER_JOB_MIB ))

JOBS=$(( MEM_JOBS < CPU_JOBS ? MEM_JOBS : CPU_JOBS ))
echo $(( JOBS > 0 ? JOBS : 1 ))
