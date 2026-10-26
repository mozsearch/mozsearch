#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A spec's pull request's renderings and semantic analysis, once it's imported
# as a try push (see tools/src/spec_forest.rs's pull requests; a tree of
# specs' import-try hook runs this after build-spec-pr and import-try-push.sh,
# with the try imports' environment, and its "Performing semantic::STEP"
# lines say on the push's page why it has no semantic analysis, if it fails;
# see tools/src/try_import.rs):
# - Its head's renderings of the Bikeshed specs it changes (render-spec-pr),
#   which the web server can't render.
# - Its head rebased onto the tree's revision (build-spec-pr's), unless its
#   changes conflict with the trunk's since its base, analyzed as the tree's
#   indexing analyzes its specs (analyze-spec-pr, with spec-analyze), and its
#   semantic delta from that (build-try-delta; see tools/src/try_delta.rs),
#   with which the rebased head's pages have its jumps and references, and
#   searches (ex: of the Gecko code citing what it changes) are of it.
# Bikeshed and spec-analyze run on the pull request's sources in a bubblewrap
# sandbox (SANDBOX=bwrap, the default): everything read-only but their outputs,
# a /tmp of their own, and no network.  SANDBOX=none runs them without (ex:
# the test trees', in a container without bubblewrap); never on a server.
#
# Usage: analyze-spec-pr.sh SPECS_TOML NAME BUILD_SPEC_PR_OUTPUT

SPECS_TOML=$1
NAME=$2
OUT=$3
SPEC=${NAME%%/*}
HEAD_REV=$(sed -n 's/^head: //p' "$OUT")
BASE=$(sed -n 's/^base: //p' "$OUT")
REBASED=$(sed -n 's/^rebased: //p' "$OUT")
CONFLICTS=$(sed -n 's/^conflicts: //p' "$OUT")

# Run a command in the sandbox, with the directories before `--` writable.
sandboxed() {
    local writable=()
    while [ "$1" != "--" ]; do
        writable+=(--bind "$1" "$1")
        shift
    done
    shift
    case "${SANDBOX:-bwrap}" in
        bwrap)
            bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp "${writable[@]}" \
                --setenv HOME /tmp --setenv TMPDIR /tmp \
                --unshare-net --unshare-pid --unshare-ipc --unshare-uts \
                --die-with-parent --new-session "$@"
            ;;
        none)
            "$@"
            ;;
        *)
            echo "ERROR: unknown SANDBOX ${SANDBOX}"
            return 1
            ;;
    esac
}

RENDERED="$INDEX_ROOT/try-rendered/$HEAD_REV"
mkdir -p "$RENDERED"
sandboxed "$RENDERED" -- render-spec-pr "$SPECS_TOML" "$GIT_ROOT" "$SPEC" "$BASE" "$HEAD_REV" "$RENDERED" ||
    echo "WARNING: Rendering its Bikeshed specs failed."

echo "Performing semantic::rebase step for $NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
if [ -z "$REBASED" ]; then
    for FILE in $CONFLICTS; do
        echo "Conflicting file: $FILE"
    done
    echo "ERROR: Its changes conflict with $SPEC's since its base, so it can't be rebased onto the tree's revision."
    exit 1
fi

echo "Performing semantic::analyze step for $NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
WORK="$INDEX_ROOT/try-analysis/$REBASED"
rm -rf "$WORK"
mkdir -p "$WORK" "$INDEX_ROOT/try-rendered/$REBASED"
sandboxed "$WORK" "$INDEX_ROOT/try-rendered/$REBASED" -- \
    analyze-spec-pr "$SPECS_TOML" "$GIT_ROOT" "$FILES_ROOT" "$INDEX_ROOT" "$SPEC" "$REBASED" "$WORK"

echo "Performing semantic::delta step for $NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
build-try-delta "$CONFIG_FILE" "$TREE_NAME" "$NAME" "$REBASED" "$WORK/analysis"
rm -rf "$WORK"
echo "Its semantic delta is $INDEX_ROOT/try-deltas/$REBASED."
