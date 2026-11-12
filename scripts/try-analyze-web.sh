#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's web analyses, as indexing makes the tree's, for its semantic
# delta (see tools/src/try_delta.rs), of the files in CHANGED (a list of
# paths, ex: those the push changes) at FILES_ROOT (its head's checkout), into
# OUT_DIR/{path}, but those OUT_DIR has already (ex: its TypeScript's):
# 1. Its JS environments' (if the tree has them; see js-environments.sh): the
#    tree's environments at its head (the tree's own, if the push changes
#    nothing their discovery reads), by js-environments over the tree's
#    files (with those it adds, but those it removes, and with the tree's URL
#    map), taking what the tree's discovery took from the files the push
#    doesn't change (js-environments' --facts); and, if the tree's are
#    analyzed (see js-environments-analyze.sh),
#    their projects documenting its files (js-environments-projects'
#    --only-files), indexed with scip-typescript, but for the files the
#    tree's TypeScript projects cover (the tree's own
#    js-environments-work/covered, as its analysis was the tree's) and those
#    OUT_DIR has; their pages' analyses go to WORK/pages.
# 2. Its JS's and pages' markup's, with js-analyze (try-analyze-js.sh), but
#    the scripts and event handlers of the pages in WORK/pages.
# 3. Its stylesheets' (try-analyze-css.sh).
# 4. Its pages' analyses merged (js-environments-pages.sh).
# 5. Its web-platform-tests' files' links (document-links, with its head's
#    links, and the tree's analysis for the files it doesn't change).
# As each one's, its web-platform-tests' custom properties are each file's
# own (by the tree's wpt_root).  Each step says when it starts.
#
# Usage: try-analyze-web.sh CONFIG_FILE TREE_NAME INDEX_ROOT FILES_ROOT CHANGED OUT_DIR

if [ $# -ne 6 ]; then
    echo "Usage: try-analyze-web.sh CONFIG_FILE TREE_NAME INDEX_ROOT FILES_ROOT CHANGED OUT_DIR" 1>&2
    exit 1
fi
CONFIG_FILE=$(realpath "$1")
TREE_NAME=$2
INDEX_ROOT=$(realpath "$3")
FILES_ROOT=$(realpath "$4")
CHANGED=$(realpath "$5")
OUT_DIR=$(realpath "$6")
export CONFIG_REPO=${CONFIG_REPO:-$(jq -r .config_repo "$CONFIG_FILE")}
export TESTS_ROOT=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].wpt_root // "-"' "$CONFIG_FILE")
SCRIPTS=$(cd "$(dirname "$0")" && pwd)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
# (The push's files at its head, but generated ones; and those it removes.)
grep -v '^__GENERATED__/' "$CHANGED" | while IFS= read -r FILE; do
    if [ -e "$FILES_ROOT/$FILE" ]; then echo "$FILE"; fi
done | LC_ALL=C sort -u > "$WORK/at-head"
grep -v '^__GENERATED__/' "$CHANGED" | while IFS= read -r FILE; do
    if [ ! -e "$FILES_ROOT/$FILE" ]; then echo "$FILE"; fi
done | LC_ALL=C sort -u > "$WORK/removed"

echo "Performing try-analyze-web::js-environments step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
RULES=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments // empty' "$CONFIG_FILE")
ANALYSIS=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments_analysis // false' "$CONFIG_FILE")
COVERED=$INDEX_ROOT/js-environments-work/covered
if [ -n "$RULES" ] && [ -s "$WORK/at-head" ]; then
    # (The environments, and web-platform-tests' links, at its head.)
    mkdir -p "$WORK/index/aliases"
    LC_ALL=C sort -u "$INDEX_ROOT/repo-files" "$WORK/at-head" |
        LC_ALL=C comm -23 - "$WORK/removed" > "$WORK/index/repo-files"
    if [ -f "$INDEX_ROOT/aliases/url-map.json" ]; then
        cp "$INDEX_ROOT/aliases/url-map.json" "$WORK/index/aliases/"
    fi
    # (The tree's, if the push changes nothing their discovery reads, as its
    # head is its stack rebased onto the indexed revision, whose files are the
    # tree's checkout's; see js-environments' --unchanged: on config8's web
    # server, discovering firefox-try's took about 30 s a push.)
    ENVIRONMENTS=(js-environments.json js-environments-links.json js-environments-files.json js-environments-paths)
    tree_environments() {
        local file tree_files
        tree_files=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].files_path // empty' "$CONFIG_FILE")
        [ -n "$tree_files" ] && [ -d "$tree_files" ] || return 1
        for file in "${ENVIRONMENTS[@]}"; do
            [ -f "$INDEX_ROOT/$file" ] || return 1
        done
        cat "$WORK/at-head" "$WORK/removed" |
            js-environments --unchanged "$tree_files" "$FILES_ROOT" "$INDEX_ROOT" "$CONFIG_REPO/$RULES"
    }
    # (Otherwise, the tree's discovery's facts for the files the push doesn't
    # change, whose sources are the tree's, rather than parsing all of them
    # again: about a third of the time; or if the index has none, or they
    # can't be read, as before the index's next indexing, the discovery.)
    if tree_environments; then
        for file in "${ENVIRONMENTS[@]}"; do
            cp "$INDEX_ROOT/$file" "$WORK/index/"
        done
    elif cat "$WORK/at-head" "$WORK/removed" > "$WORK/changed" &&
            [ -f "$INDEX_ROOT/js-environments-facts.json" ] &&
            js-environments --facts "$INDEX_ROOT/js-environments-facts.json" --changed "$WORK/changed" \
                "$FILES_ROOT" "$WORK/index" "$CONFIG_REPO/$RULES" > "$WORK/js-environments.log"; then
        echo "The push's JS environments took the tree's discovery's facts for the files it doesn't change."
    else
        js-environments "$FILES_ROOT" "$WORK/index" "$CONFIG_REPO/$RULES" > "$WORK/js-environments.log"
    fi
    if [ "$ANALYSIS" = "true" ] && [ ! -f "$COVERED" ]; then
        echo "WARNING: The tree's JS environments' analysis has no list of the files it left out, so the push's environments' files are js-analyze's."
    elif [ "$ANALYSIS" = "true" ]; then
        { cat "$COVERED"; (cd "$OUT_DIR" && find . -type f | sed 's#^\./##'); } > "$WORK/covered"
        JS_ENVIRONMENTS_INDEX=$WORK/index JS_ENVIRONMENTS_WORK=$WORK/work \
            JS_ENVIRONMENTS_ANALYSIS=$OUT_DIR JS_ENVIRONMENTS_PAGES=$WORK/pages \
            JS_ENVIRONMENTS_COVERED=$WORK/covered JS_ENVIRONMENTS_ONLY_FILES=$WORK/at-head \
            FILES_ROOT=$FILES_ROOT INDEX_ROOT=$INDEX_ROOT \
            "$SCRIPTS/js-environments-analyze.sh" "$CONFIG_FILE" "$TREE_NAME"
    fi
fi

echo "Performing try-analyze-web::js step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
JS_ENVIRONMENTS_PAGES=$WORK/pages "$SCRIPTS/try-analyze-js.sh" "$INDEX_ROOT" "$FILES_ROOT" "$CHANGED" "$OUT_DIR"

echo "Performing try-analyze-web::css step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
"$SCRIPTS/try-analyze-css.sh" "$INDEX_ROOT" "$FILES_ROOT" "$CHANGED" "$OUT_DIR"

echo "Performing try-analyze-web::pages step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
JS_ENVIRONMENTS_PAGES=$WORK/pages JS_ENVIRONMENTS_ANALYSIS=$OUT_DIR INDEX_ROOT=$INDEX_ROOT \
    "$SCRIPTS/js-environments-pages.sh" "$CONFIG_FILE" "$TREE_NAME"

echo "Performing try-analyze-web::links step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
if [ -f "$WORK/index/js-environments-links.json" ]; then
    document-links "$INDEX_ROOT" --links "$WORK/index/js-environments-links.json" \
        --analysis "$OUT_DIR" --base "$INDEX_ROOT/analysis" --only "$WORK/at-head"
fi
