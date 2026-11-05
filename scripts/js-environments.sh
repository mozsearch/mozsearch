#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: js-environments.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# The tree's JS environments (the globals its scripts run in, with which other
# scripts; see tools/src/bin/js-environments.rs), if its config names its
# rules (`js_environments`, a path in the config repo): js-environments.json,
# and each file's for the navigation panel (js-environments-files.json, which
# per-file-info.toml ingests).  It runs after process-chrome-map, whose URL
# map resolves chrome:// and resource:// URLs, and before
# js-environments-analyze.sh, which analyzes them.
RULES=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments // empty' "$CONFIG_FILE")
if [ -z "$RULES" ]; then
    echo "The tree has no JS environment rules."
    exit 0
fi
js-environments "$FILES_ROOT" "$INDEX_ROOT" "$CONFIG_REPO/$RULES"
