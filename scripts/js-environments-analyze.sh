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
# leaves out the files it analyzes.  (JS_ENVIRONMENTS_FLAVORS, a
# comma-separated list, limits it to some flavors.)
RULES=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments // empty' "$CONFIG_FILE")
ANALYSIS=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments_analysis // false' "$CONFIG_FILE")
if [ -z "$RULES" ] || [ "$ANALYSIS" != "true" ] || [ ! -f "$INDEX_ROOT/js-environments.json" ]; then
    echo "The tree's JS environments aren't analyzed."
    exit 0
fi
WORK=$INDEX_ROOT/js-environments-work
rm -rf "$WORK"
mkdir -p "$WORK/scip"
JOBS=${JS_ENVIRONMENTS_JOBS:-$(( $(nproc) > 2 ? $(nproc) / 2 : 1 ))}

(cd "$INDEX_ROOT/analysis" && find . -type f | sed 's#^\./##') > "$WORK/covered"
js-environments-projects "$FILES_ROOT" "$INDEX_ROOT" "$CONFIG_REPO/$RULES" "$WORK" \
    --covered "$WORK/covered" --batch-size "${JS_ENVIRONMENTS_BATCH:-50}" ${JS_ENVIRONMENTS_FLAVORS:+--flavors "$JS_ENVIRONMENTS_FLAVORS"}

# Each batch's index, in its kind's analysis directory.  Files bigger than
# 200 KB aren't documented (ex: SpiderMonkey's and web-platform-tests'
# generated data, ex: a 400 KB object literal took TypeScript's checker 65 s,
# rechecking it for each of its properties; js-analyze analyzes them), and a
# batch which fails (ex: a test nesting so deep that TypeScript's binder's
# recursion overflows its stack, killing the process) or takes longer than
# JS_ENVIRONMENTS_TIMEOUT (900 s by default) has its projects indexed one at
# a time, each with as long, so that only the failing ones' files are
# js-analyze's.
index_projects() {
    local name=$1
    local kind=$2
    shift 2
    # (Its --cwd is FILES_ROOT as the projects name their files, though it's a
    # symbolic link, whose real path the process's working directory is.)
    (cd "$FILES_ROOT" && timeout "${JS_ENVIRONMENTS_TIMEOUT:-900}" scip-typescript index \
         --cwd "$FILES_ROOT" --no-progress-bar --stable-type-ordering --classic-scripts --hooks \
         --max-file-byte-size 200kb --output "$WORK/scip/$name.scip" "$@") \
        > "$WORK/scip/$name.log" 2>&1 || return 1
    scip-indexer "$CONFIG_FILE" "$TREE_NAME" --subtree-name typescript --subtree-root "" \
        --analysis-root "$WORK/analysis-$kind" --shims __jsenv_shims__ --no-leftovers \
        "$WORK/scip/$name.scip" > "$WORK/scip/$name.indexer.log" 2>&1
    rm -f "$WORK/scip/$name.scip"
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
    echo "WARNING: scip-typescript failed for $name, so its projects are indexed one at a time:"
    tail -3 "$WORK/scip/$name.log"
    local project
    for project in "${projects[@]}"; do
        local one
        one=$name.$(basename "$project" .json)
        if ! index_projects "$one" "$kind" "$project"; then
            echo "WARNING: scip-typescript failed for $one ($project), whose files are js-analyze's."
        fi
    done
}
export -f index_projects index_batch
export WORK FILES_ROOT CONFIG_FILE TREE_NAME
# (The costliest first; see js-environments-projects.)
parallel -j "$JOBS" --halt now,fail=1 index_batch < "$WORK/batches.lst"
grep -h "^unexpected error" "$WORK"/scip/*.log | cut -c1-200 | sort | uniq -c | sort -rn | head -20 || true

# The kinds' analyses, merged.
(cd "$WORK" && find . -path './analysis-*' -type f | sed 's#^\./analysis-[^/]*/##' | sort -u) > "$WORK/analyzed"
echo "Merging the analysis of $(wc -l < "$WORK/analyzed") files of JS environments."
(cd "$WORK" && parallel --pipe -N 500 --halt now,fail=1 merge-analyses --batch "$INDEX_ROOT/analysis" < analyzed)
rm -rf "$WORK"/analysis-* "$WORK/projects"
