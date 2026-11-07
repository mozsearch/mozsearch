#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's Python analysis, as the tree's `python` SCIP subtree's (see
# scip-analyze.sh), for its semantic delta (see tools/src/try_delta.rs): of
# the `.py` files in CHANGED at FILES_ROOT (its head's checkout), but those
# OUT_DIR has already, into OUT_DIR/{path}.  Each CONFIG is one of the pyright
# projects the tree's scip-python indexes (a pyright configuration file,
# whose include, exclude and extraPaths are relative to FILES_ROOT, ex: its
# web-platform-tests' and the rest's), whose files the push changes (those
# under its includes, but not its excludes, by their paths) are indexed with
# it, with only them included, so that their imports resolve as the tree's
# do; scip-python's documents of the files they import are left out.  (On
# firefox at config8's 2026-11-06 revision, 2 files took 31 s, 0.9 GB.)
#
# Usage: try-analyze-python.sh CONFIG_FILE TREE_NAME FILES_ROOT CHANGED OUT_DIR CONFIG...

if [ $# -lt 6 ]; then
    echo "Usage: try-analyze-python.sh CONFIG_FILE TREE_NAME FILES_ROOT CHANGED OUT_DIR CONFIG..." 1>&2
    exit 1
fi
CONFIG_FILE=$(realpath "$1")
TREE_NAME=$2
FILES_ROOT=$(realpath "$3")
CHANGED=$(realpath "$4")
OUT_DIR=$(realpath "$5")
shift 5

SUBTREE_ROOT=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].scip_subtrees.python.subtree_root // empty' "$CONFIG_FILE")
if [ -z "$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].scip_subtrees.python // empty' "$CONFIG_FILE")" ]; then
    echo "The tree's Python isn't analyzed."
    exit 0
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"; rm -f "$FILES_ROOT"/.try-pyright-*.json' EXIT
N=0
for CONFIG in "$@"; do
    N=$((N + 1))
    # (The project's files the push changes, and its configuration with only
    # them included.)
    python3 -I - "$CONFIG" "$FILES_ROOT" "$CHANGED" "$OUT_DIR" \
        "$FILES_ROOT/.try-pyright-$$-$N.json" "$WORK/files-$N" <<'PY'
import json, os, sys
config_path, files_root, changed, out_dir, out_config, out_files = sys.argv[1:]
config = json.load(open(config_path))
def under(path, dirs):
    return any(d in (".", "") or path == d or path.startswith(d.rstrip("/") + "/") for d in dirs)
files = []
for path in open(changed).read().split("\n"):
    if (path.endswith(".py") and os.path.isfile(os.path.join(files_root, path))
            and not os.path.exists(os.path.join(out_dir, path))
            and under(path, config.get("include", ["."]))
            and not under(path, config.get("exclude", []))):
        files.append(path)
config["include"] = files
json.dump(config, open(out_config, "w"))
open(out_files, "w").write("".join(f + "\n" for f in files))
PY
    if [ ! -s "$WORK/files-$N" ]; then
        continue
    fi
    echo "Analyzing the push's $(wc -l < "$WORK/files-$N") Python files of $CONFIG."
    (cd "$FILES_ROOT" && NODE_OPTIONS="--max-old-space-size=10240" scip-python index --cwd "$FILES_ROOT" \
         --show-progress-rate-limit 0 --project-name python \
         --config "$FILES_ROOT/.try-pyright-$$-$N.json" --output "$WORK/$N.scip") > "$WORK/$N.log" 2>&1 ||
        { tail -5 "$WORK/$N.log"; echo "WARNING: scip-python failed for the push's files of $CONFIG."; continue; }
    scip-indexer "$CONFIG_FILE" "$TREE_NAME" --subtree-name python --subtree-root "$SUBTREE_ROOT" \
        --analysis-root "$WORK/analysis-$N" --files-root "$FILES_ROOT" --no-leftovers "$WORK/$N.scip"
    # (Only the push's files', not those of the files they import.)
    while IFS= read -r FILE; do
        if [ -f "$WORK/analysis-$N/$FILE" ]; then
            mkdir -p "$(dirname "$OUT_DIR/$FILE")"
            cp "$WORK/analysis-$N/$FILE" "$OUT_DIR/$FILE"
        fi
    done < "$WORK/files-$N"
done
