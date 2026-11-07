#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: js-environments-pages.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# The analyses of the pages whose inline scripts and event handlers their JS
# environments' analysis has (js-environments-pages/, see
# js-environments-analyze.sh), each merged into the tree's analysis with
# js-analyze's of its markup and styles, if it wrote one (html-analyze.sh's,
# or js-analyze.sh's for XUL; they leave out those pages' scripts and
# handlers), with merge-analyses, as two platforms' (the analysis-markup and
# analysis-scripts directories of each page's files).
PAGES=$INDEX_ROOT/js-environments-pages
if [ ! -d "$PAGES" ]; then
    echo "There are no pages' JS environments' analyses."
    exit 0
fi
MERGE=$INDEX_ROOT/js-environments-pages-merge
rm -rf "$MERGE"
mkdir -p "$MERGE/merged" "$MERGE/analysis-markup"
mv "$PAGES" "$MERGE/analysis-scripts"
(cd "$MERGE/analysis-scripts" && find . -type f | sed 's#^\./##' | LC_ALL=C sort) > "$MERGE/pages"
(cd "$INDEX_ROOT/analysis" && while IFS= read -r page; do
     if [ -f "$page" ]; then printf '%s\n' "$page"; fi
 done < "$MERGE/pages") > "$MERGE/markup"
(cd "$INDEX_ROOT/analysis" && xargs -r -d '\n' -a "$MERGE/markup" cp --parents -t "$MERGE/analysis-markup")
echo "Merging $(wc -l < "$MERGE/pages") pages' JS environments' analyses, $(wc -l < "$MERGE/markup") with their markup's."
(cd "$MERGE" && parallel --pipe -N 500 --halt now,fail=1 merge-analyses --batch "$MERGE/merged" < pages)
cp -a "$MERGE/merged/." "$INDEX_ROOT/analysis/"
rm -rf "$MERGE"
