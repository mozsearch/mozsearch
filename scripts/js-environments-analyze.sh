#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: js-environments-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# The tree's JS environments' analysis (see js-environments.sh, which finds
# them, and tools/src/js_environments/projects.rs), if its config says so
# (`js_environments_analysis`): with scip-typescript, projects of environments
# sharing a context (with --classic-scripts, so that the scripts' top-level
# declarations are the globals the others see, and their tests modules), in
# batches by environment kind (of JS_ENVIRONMENTS_BATCH projects, 50 by
# default, as each process loads its global's types once), in parallel
# (JS_ENVIRONMENTS_JOBS processes, by default half the cores), documenting
# each script once per kind, but those whose analysis there is already (ex:
# TypeScript projects'), with --hooks (the names harnesses call their tests'
# functions by are hooks, whose definitions are the tests'); each kind's
# analysis (with scip-indexer, whose --shims makes the uses of a flavor's
# scope's members uses of their definitions) merged as platforms' into the
# tree's analysis (merge-analyses), so that a script in several kinds of
# environments has each one's definitions.  js-analyze, which runs after it,
# leaves out the files it analyzes.  Pages' inline scripts and event handlers
# are analyzed as files of their own (js-environments-projects'
# __jsenv_inline__/<page>.js, with the pages' positions), whose analyses
# scip-indexer's --inline-pages makes the pages', merged into
# js-environments-pages/ rather than the tree's analysis, as js-analyze's of
# their markup (and its styles) is to come: it leaves out the scripts and
# handlers of the pages there, and js-environments-pages.sh then merges both.
# (JS_ENVIRONMENTS_FLAVORS, a comma-separated list, limits it to some
# flavors.)
#
# A try push's analysis (see try-analyze-web.sh) gives it its own
# directories, rather than the tree's index's: JS_ENVIRONMENTS_INDEX (with its
# js-environments.json), JS_ENVIRONMENTS_WORK, JS_ENVIRONMENTS_ANALYSIS (where
# the scripts' analyses go), JS_ENVIRONMENTS_PAGES (the pages'),
# JS_ENVIRONMENTS_COVERED (the files whose analysis it leaves out, rather
# than the tree's analysis's), and JS_ENVIRONMENTS_ONLY_FILES, the only files
# it documents (see js-environments-projects' --only-files); FILES_ROOT is
# then its checkout.
RULES=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments // empty' "$CONFIG_FILE")
ANALYSIS=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments_analysis // false' "$CONFIG_FILE")
ENV_INDEX=${JS_ENVIRONMENTS_INDEX:-$INDEX_ROOT}
if [ -z "$RULES" ] || [ "$ANALYSIS" != "true" ] || [ ! -f "$ENV_INDEX/js-environments.json" ]; then
    echo "The tree's JS environments aren't analyzed."
    exit 0
fi
WORK=${JS_ENVIRONMENTS_WORK:-$INDEX_ROOT/js-environments-work}
OUT=${JS_ENVIRONMENTS_ANALYSIS:-$INDEX_ROOT/analysis}
PAGES=${JS_ENVIRONMENTS_PAGES:-$INDEX_ROOT/js-environments-pages}
rm -rf "$WORK" "$PAGES"
mkdir -p "$WORK/scip"
JOBS=${JS_ENVIRONMENTS_JOBS:-$(( $(nproc) > 2 ? $(nproc) / 2 : 1 ))}

if [ -n "${JS_ENVIRONMENTS_COVERED:-}" ]; then
    cp "$JS_ENVIRONMENTS_COVERED" "$WORK/covered"
else
    (cd "$OUT" && find . -type f | sed 's#^\./##') > "$WORK/covered"
fi
js-environments-projects "$FILES_ROOT" "$ENV_INDEX" "$CONFIG_REPO/$RULES" "$WORK" \
    --covered "$WORK/covered" --batch-size "${JS_ENVIRONMENTS_BATCH:-50}" \
    ${JS_ENVIRONMENTS_FLAVORS:+--flavors "$JS_ENVIRONMENTS_FLAVORS"} \
    ${JS_ENVIRONMENTS_ONLY_FILES:+--only-files "$JS_ENVIRONMENTS_ONLY_FILES"}

# Each batch's index, in its kind's analysis directory, of files of any size
# (but those bigger than JS_ENVIRONMENTS_MAX_FILE_SIZE, a scip-typescript
# --max-file-byte-size, if it's given, which js-analyze analyzes): with the
# 200 KB limit there was, mozilla-central's 120 batches took 47 s on 16
# processes on a 64-core machine, and 52 s without, documenting 76 more files
# (SpiderMonkey's and web-platform-tests' generated data, ex: wasm's .wast.js
# tests, 72 MB; 9% more occurrences).  (The limit was for scip-typescript's
# checking an object literal again for each of its properties, ex: 65 s for a
# 400 KB one, which mozsearch's fork no longer does.)  A project TypeScript
# fails for (ex: a test nesting so deep that its binder's
# recursion overflows its stack) has no documents (--skip-failed-projects), so
# that its files are js-analyze's, and a batch which fails otherwise (ex: its
# process killed), takes longer than JS_ENVIRONMENTS_TIMEOUT (900 s by
# default) or whose index scip-indexer fails for (ex: a string which isn't
# UTF-8, which scip-indexer can't read any of the index for: before
# scip-typescript's fork escaped lone surrogates, 3 of firefox's 120 batches)
# has its projects indexed one at a time, each with as long, so that only the
# failing ones' files are js-analyze's.
index_projects() {
    local name=$1
    local kind=$2
    shift 2
    # (Its --cwd is FILES_ROOT as the projects name their files, though it's a
    # symbolic link, whose real path the process's working directory is.)
    (cd "$FILES_ROOT" && timeout "${JS_ENVIRONMENTS_TIMEOUT:-900}" scip-typescript index \
         --cwd "$FILES_ROOT" --no-progress-bar --stable-type-ordering --classic-scripts --hooks \
         --skip-failed-projects --max-file-byte-size "${JS_ENVIRONMENTS_MAX_FILE_SIZE:-0}" \
         --output "$WORK/scip/$name.scip" "$@") \
        > "$WORK/scip/$name.log" 2>&1 || return 1
    local status=0
    scip-indexer "$CONFIG_FILE" "$TREE_NAME" --subtree-name typescript --subtree-root "" \
        --analysis-root "$WORK/analysis-$kind" --shims __jsenv_shims__ --no-leftovers \
        --inline-pages "$WORK/__jsenv_inline__" --files-root "$FILES_ROOT" \
        "$WORK/scip/$name.scip" > "$WORK/scip/$name.indexer.log" 2>&1 || status=1
    rm -f "$WORK/scip/$name.scip"
    return $status
}
index_batch() {
    local batch=$1
    local name
    name=$(basename "$batch" .lst)
    local kind=${name%.*}
    local projects
    mapfile -t projects < "$batch"
    if index_projects "$name" "$kind" "${projects[@]}"; then
        return 0
    fi
    echo "WARNING: scip-typescript or scip-indexer failed for $name, so its projects are indexed one at a time:"
    tail -n 3 "$WORK/scip/$name.log" "$WORK/scip/$name.indexer.log" 2>/dev/null || true
    local project
    for project in "${projects[@]}"; do
        local one
        one=$name.$(basename "$project" .json)
        if ! index_projects "$one" "$kind" "$project"; then
            echo "WARNING: scip-typescript or scip-indexer failed for $one ($project), whose files are js-analyze's."
        fi
    done
}
export -f index_projects index_batch
export WORK FILES_ROOT CONFIG_FILE TREE_NAME
# (The costliest first; see js-environments-projects.)
parallel -j "$JOBS" --halt now,fail=1 index_batch < "$WORK/batches.lst"
grep -h "^unexpected error" "$WORK"/scip/*.log | cut -c1-200 | sort | uniq -c | sort -rn | head -20 || true

# The kinds' analyses, merged: the scripts' into the tree's analysis, and the
# pages' into js-environments-pages/.
(cd "$WORK" && find . -path './analysis-*' -type f | sed 's#^\./analysis-[^/]*/##' | LC_ALL=C sort -u) > "$WORK/analyzed"
LC_ALL=C sort -u "$WORK/inline-pages" > "$WORK/inline-pages.sorted"
LC_ALL=C comm -12 "$WORK/analyzed" "$WORK/inline-pages.sorted" > "$WORK/analyzed-pages"
LC_ALL=C comm -23 "$WORK/analyzed" "$WORK/inline-pages.sorted" > "$WORK/analyzed-scripts"
echo "Merging the analysis of $(wc -l < "$WORK/analyzed-scripts") files and $(wc -l < "$WORK/analyzed-pages") pages of JS environments."
(cd "$WORK" && parallel --pipe -N 500 --halt now,fail=1 merge-analyses --batch "$OUT" < analyzed-scripts)
mkdir -p "$PAGES"
(cd "$WORK" && parallel --pipe -N 500 --halt now,fail=1 merge-analyses --batch "$PAGES" < analyzed-pages)
rm -rf "$WORK"/analysis-* "$WORK/projects" "$WORK/__jsenv_inline__"
