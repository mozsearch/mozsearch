#!/usr/bin/env bash

set -eu
set -o pipefail

# The tools' unit tests (the flake's mozsearch-tools-tests; see
# nix/mozsearch/tools.nix), built and run in the background while the
# container's suite builds its index (see the Makefile's check-all), as
# installing the tools doesn't build them: `start` starts them, from the
# flake in FLAKE_DIR (/vagrant by default), and `wait` waits for them, and
# fails if they did, with their failures.
#
# Usage: unit-tests-background.sh start [FLAKE_DIR]
#        unit-tests-background.sh wait

LOG=~/unit-tests.log
STATUS=~/unit-tests.status

case "${1:-}" in
    start)
        FLAKE_DIR=${2:-/vagrant}
        rm -f "$STATUS"
        # (In a session of its own, with its output in its log, so that make
        # doesn't wait for it.)
        setsid bash -c "sudo -i nix build '$FLAKE_DIR#mozsearch-tools-tests' --no-link \
            --accept-flake-config --print-build-logs > '$LOG' 2>&1; echo \$? > '$STATUS'" \
            < /dev/null > /dev/null 2>&1 &
        echo "Building and running the tools' unit tests in the background (see $LOG)."
        ;;
    wait)
        if [ ! -e "$STATUS" ] && ! pgrep -f "^bash -c sudo -i nix build .*#mozsearch-tools-tests" > /dev/null; then
            echo "The tools' unit tests weren't started."
            exit 1
        fi
        while [ ! -e "$STATUS" ]; do
            sleep 5
        done
        if [ "$(cat "$STATUS")" != "0" ]; then
            echo "The tools' unit tests failed (see $LOG):"
            grep -E "panicked at|FAILED|^tools-test> failures:|error(\[|:)" "$LOG" | tail -40 || true
            exit 1
        fi
        echo "The tools' unit tests passed$(grep -oE "test result: ok\. [0-9]+ passed" "$LOG" | head -1 | sed 's/^/ (/; s/$/)/')."
        ;;
    *)
        echo "Usage: $0 start [FLAKE_DIR] | wait"
        exit 1
        ;;
esac
