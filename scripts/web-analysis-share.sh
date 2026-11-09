#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A tree's web analyses (its web analysis steps' outputs: scip-analyze's,
# js-environments-analyze's, js-, html- and css-analyze's, js-environments-
# pages' and document-links') from another tree's indexing, rather than its
# own steps, if its config names the other tree (`web_analysis_from`, ex:
# firefox-try's, firefox-disco's, the same revision: of the platforms' builds
# they differ in, the steps read nothing), and if they would be the same: the
# other tree's (the producer's) mkindex.sh snapshots its analyses after the
# steps, with what they depend on (see `key`), and the tree's (the
# consumer's) restores them, once its own js-environments step has made the
# environments (which the URL map its build's chrome maps make could change),
# if what they depend on is the same, and if it has the same analyses before
# the steps of the files the steps analyze (but `.inc` files', which only
# js-analyze reads, whose analyses its js-analyze step then makes, ex: of
# C++ another platform's build compiled), rather than running the steps (on
# config8's 2026-11-09 run, firefox-try's took 4.7 minutes, its crossref's
# inputs, the indexing's last; its analyses of a sample of 4,500 of the
# files were firefox-disco's, byte for byte).  Otherwise (ex: the producer's
# steps failed, or it wasn't indexed first), the consumer's steps run, and it
# says why, in a warning.  mkindex.sh calls it:
#
#   web-analysis-share.sh list CONFIG_FILE TREE_NAME
#     before the steps (and idl-analyze, which runs alongside them), lists
#     the tree's analysis files, with their modification times;
#   web-analysis-share.sh snapshot CONFIG_FILE TREE_NAME [FAILED]
#     after them, for the trees whose analyses are its, copies the analysis
#     files the steps made (those not listed before, or modified since, but
#     idl-analyze's), unless the steps failed (FAILED isn't empty) or
#     modified any analysis there was before them (ex: the build's), with
#     the files' list and its analyses before the steps of the files they
#     analyze (and the files js-environments-analyze found analyzed, for
#     try-analyze-web.sh's js-environments-work/covered);
#   web-analysis-share.sh restore CONFIG_FILE TREE_NAME PRODUCER
#     after the tree's js-environments step, restores PRODUCER's snapshot, if
#     it can (listing in INDEX_ROOT/web-analysis-js-files the files its
#     js-analyze step still analyzes), and exits with 1 if it doesn't.
#
# The snapshots are in WEB_ANALYSIS_DIR (by default the working directory's
# tmp/web-analysis/, which indexer-run.sh empties when it starts and ends,
# on the index's filesystem, whose hard links make the last consumer's
# restoring quick).  (The snapshot is a copy, as the producer's later steps
# rewrite some analysis files in place, ex: replace-aliases.py.)

if [ $# -lt 3 ]; then
    echo "Usage: web-analysis-share.sh list|snapshot|restore CONFIG_FILE TREE_NAME [FAILED|PRODUCER]" 1>&2
    exit 1
fi

ACTION=$1
CONFIG_FILE=$(realpath "$2")
TREE_NAME=$3

DIR=${WEB_ANALYSIS_DIR:-${WORKING:+$WORKING/tmp/web-analysis}}
if [ -z "$DIR" ]; then
    echo "There's no WEB_ANALYSIS_DIR (or WORKING), so the tree's web analyses aren't shared."
    exit 1
fi
mkdir -p "$DIR"
BEFORE=$DIR/$TREE_NAME.before
export LC_ALL=C

# The tree's analysis files, sorted, with their modification times, and the
# paths of those before the steps.
listing() {
    (cd "$INDEX_ROOT/analysis" && find . -type f -printf '%P\t%T@\n') | sort
}
before_paths() {
    cut -f1 "$BEFORE" | sort -u
}

# The first few lines of FILE, on one line.
examples() {
    sed -n '1,3p' "$1" | paste -sd ' '
}

# What the steps' analyses depend on, but the files they analyze's analyses
# before them: the files (the revision, any local changes and the tree's
# lists of its files), the tree's settings for the steps (the names of its
# SCIP indexes, whose leftovers' analysis files are named after them), its
# JS environment rules, its JS environments (js-environments' outputs) and
# js-environments-analyze.sh's environment variables.
key() {
    local revision
    revision=$(git -C "$FILES_ROOT" rev-parse HEAD 2>/dev/null) || return 1
    echo "revision $revision"
    echo "changes $(git -C "$FILES_ROOT" diff --no-ext-diff --binary HEAD | md5sum | cut -d' ' -f1)"
    local file
    for file in repo-files js-files html-files css-files js-environments.json \
                js-environments-links.json js-environments-files.json js-environments-paths; do
        if [ -f "$INDEX_ROOT/$file" ]; then
            echo "$file $(md5sum < "$INDEX_ROOT/$file" | cut -d' ' -f1)"
        else
            echo "$file none"
        fi
    done
    echo "settings $(jq -cS --arg tree "$TREE_NAME" '.trees[$tree] | {
        wpt_root, js_environments, js_environments_analysis,
        scip_subtrees: ((.scip_subtrees // {}) | map_values({
            subtree_root,
            index: (.scip_index_path // "" | split("/") | last),
            index2: (.scip_index_path2 // "" | split("/") | last)}))}' "$CONFIG_FILE")"
    local rules
    rules=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].js_environments // empty' "$CONFIG_FILE")
    if [ -n "$rules" ]; then
        echo "rules $(md5sum < "$CONFIG_REPO/$rules" | cut -d' ' -f1)"
    fi
    echo "environment ${JS_ENVIRONMENTS_BATCH:-}/${JS_ENVIRONMENTS_TIMEOUT:-}/${JS_ENVIRONMENTS_MAX_FILE_SIZE:-}/${JS_ENVIRONMENTS_FLAVORS:-}/${JS_ENVIRONMENTS_ONLY_FILES:-}"
}

# The analysis files before the steps of FILES (and of the files the steps
# analyze), from the listing before them.
web_before() {
    cat "$INDEX_ROOT/js-files" "$INDEX_ROOT/html-files" "$INDEX_ROOT/css-files" "$1" 2>/dev/null |
        sort -u | comm -12 <(before_paths) -
}

case "$ACTION" in
list)
    listing > "$BEFORE.tmp"
    mv "$BEFORE.tmp" "$BEFORE"
    echo "web-analysis-share: $(wc -l < "$BEFORE") analysis files before the web analysis steps."
    ;;
snapshot)
    FAILED=${4:-}
    SNAP=$DIR/$TREE_NAME
    rm -rf "$SNAP" "$SNAP.refused"
    # (Why there's none, for the consumers' warnings.)
    refuse() {
        echo "web-analysis-share: no snapshot of $TREE_NAME's web analyses, as $1."
        echo "$1" > "$SNAP.refused"
        rm -rf "$SNAP" "$BEFORE"
        exit 0
    }
    if [ -n "$FAILED" ]; then
        refuse "$TREE_NAME's web analysis steps failed"
    fi
    if [ ! -f "$BEFORE" ]; then
        refuse "$TREE_NAME's analysis files weren't listed before its web analysis steps (ex: they were skipped)"
    fi
    if ! KEY=$(key); then
        refuse "$TREE_NAME's files aren't a git checkout's"
    fi
    START=$SECONDS
    mkdir -p "$SNAP/analysis"
    # (The paths of the files the steps made or modified, but idl-analyze's.)
    cat "$INDEX_ROOT/idl-files" "$INDEX_ROOT/webidl-files" 2>/dev/null | sort -u > "$SNAP/idl"
    listing | comm -13 "$BEFORE" - | cut -f1 | sort -u | comm -23 - "$SNAP/idl" > "$SNAP/files"
    before_paths | comm -12 - "$SNAP/files" > "$SNAP/modified"
    if [ -s "$SNAP/modified" ]; then
        refuse "$TREE_NAME's web analysis steps modified analysis files there were before them (ex: $(examples "$SNAP/modified"))"
    fi
    if ! (cd "$SNAP/analysis" &&
              xargs -r -d '\n' -a "$SNAP/files" dirname | sort -u | xargs -r -d '\n' mkdir -p &&
              cd "$INDEX_ROOT/analysis" &&
              xargs -r -d '\n' -a "$SNAP/files" -n 1000 -P "$(nproc)" cp --parents -t "$SNAP/analysis"); then
        refuse "copying $TREE_NAME's analysis files failed"
    fi
    web_before "$SNAP/files" > "$SNAP/before"
    if [ -f "$INDEX_ROOT/js-environments-work/covered" ]; then
        sort -u "$INDEX_ROOT/js-environments-work/covered" | comm -23 - <(before_paths) |
            comm -23 - "$SNAP/idl" > "$SNAP/covered"
    fi
    echo "$KEY" > "$SNAP/key"
    jq -r --arg tree "$TREE_NAME" '.trees | to_entries[] | select(.value.web_analysis_from == $tree) | .key' \
        "$CONFIG_FILE" > "$SNAP/consumers"
    rm -f "$SNAP/idl" "$SNAP/modified" "$BEFORE"
    touch "$SNAP/complete"
    echo "web-analysis-share: a snapshot of $(wc -l < "$SNAP/files") analysis files ($(du -sm "$SNAP/analysis" | cut -f1) MB) for $(paste -sd ' ' "$SNAP/consumers"), in $((SECONDS - START)) s."
    ;;
restore)
    PRODUCER=$4
    SNAP=$DIR/$PRODUCER
    # (The files its js-analyze step still analyzes, if its web analyses are
    # the producer's; see mkindex.sh.)
    OWN=$INDEX_ROOT/web-analysis-js-files
    KEPT=$DIR/$TREE_NAME.kept
    TAKEN=$DIR/$TREE_NAME.taken
    rm -f "$OWN"
    refuse() {
        echo "warning: Tree '$TREE_NAME': its web analyses aren't $PRODUCER's, as $1, so its web analysis steps make them."
        rm -f "$BEFORE" "$DIR/$TREE_NAME.existing" "$DIR/$TREE_NAME.differing" "$OWN" "$KEPT" "$TAKEN"
        exit 1
    }
    if [ ! -f "$SNAP/complete" ]; then
        if [ -f "$SNAP.refused" ]; then
            refuse "there's no snapshot of them: $(cat "$SNAP.refused")"
        fi
        refuse "there's no snapshot of them (ex: $PRODUCER wasn't indexed first, or failed)"
    fi
    if ! grep -qx "$TREE_NAME" "$SNAP/consumers"; then
        refuse "their snapshot isn't for it"
    fi
    if [ ! -f "$BEFORE" ]; then
        refuse "its analysis files weren't listed before its web analysis steps"
    fi
    if ! KEY=$(key); then
        refuse "its files aren't a git checkout's"
    fi
    DIFFERENT=$(diff <(cat "$SNAP/key") <(echo "$KEY") | grep '^>' | cut -c3- | cut -d' ' -f1 | paste -sd ' ' || true)
    if [ -n "$DIFFERENT" ]; then
        refuse "what they depend on differs: $DIFFERENT"
    fi
    # The files the steps analyze whose analyses before them differ, which
    # can only be files js-analyze alone reads (`.inc` files, which neither
    # the SCIP indexes nor the JS environments do, ex: C++'s, which the
    # producer's build of another platform compiled, whose analysis its
    # js-analyze then left as it was: on config8's 2026-11-09 run, 34 of
    # firefox-try's files, for firefox-disco's Windows, macOS and Android
    # builds), as it analyzes each file alone, and skips those analyzed
    # already: the tree's js-analyze step analyzes those the producer had
    # analyses of, and keeps its analyses of those it has (theirs, if the
    # producer's js-analyze made any, aren't copied over them).
    DIFFERING=$DIR/$TREE_NAME.differing
    diff "$SNAP/before" <(web_before "$SNAP/files") | grep '^[<>]' | sort -k2 > "$DIFFERING" || true
    OTHERS=$(cut -c3- "$DIFFERING" | grep -cv '\.inc$' || true)
    if [ "$OTHERS" != 0 ]; then
        refuse "$OTHERS of the files they analyze had other analyses before them (ex: $(cut -c3- "$DIFFERING" | grep -v '\.inc$' | examples /dev/stdin))"
    fi
    sed -n 's/^< //p' "$DIFFERING" | sort -u | comm -12 - <(sort -u "$INDEX_ROOT/js-files") > "$OWN"
    sed -n 's/^> //p' "$DIFFERING" | sort -u > "$KEPT"
    rm -f "$DIFFERING"
    # (The snapshot's analysis files it takes: all of them, but those it
    # keeps its own of.)
    comm -23 "$SNAP/files" "$KEPT" > "$TAKEN"
    EXISTING=$DIR/$TREE_NAME.existing
    listing | cut -f1 | sort -u | comm -12 - "$TAKEN" > "$EXISTING"
    if [ -s "$EXISTING" ]; then
        refuse "it has $(wc -l < "$EXISTING") of their analysis files already (ex: $(examples "$EXISTING"))"
    fi
    rm -f "$EXISTING"
    START=$SECONDS
    # (Hard links, if it's the last consumer, as the snapshot isn't needed
    # then, whose later steps can rewrite them, and file by file if it keeps
    # any of their files' analyses.  If copying them fails, those it copied
    # are removed, as its steps then make them, and js-environments-analyze
    # merges its analyses into those there are.)
    REMAINING=$(grep -vx "$TREE_NAME" "$SNAP/consumers" || true)
    copy() {
        local link=
        if [ -z "$REMAINING" ]; then
            link=l
        fi
        if ! cmp -s "$TAKEN" "$SNAP/files"; then
            (cd "$SNAP/analysis" &&
                 xargs -r -d '\n' -a "$TAKEN" -n 1000 cp -a$link --parents -t "$INDEX_ROOT/analysis/")
            return
        fi
        if [ -n "$link" ]; then
            cp -al "$SNAP/analysis/." "$INDEX_ROOT/analysis/" 2>/dev/null && return 0
        fi
        cp -a "$SNAP/analysis/." "$INDEX_ROOT/analysis/"
    }
    if ! copy; then
        (cd "$INDEX_ROOT/analysis" && xargs -r -d '\n' -a "$TAKEN" rm -f)
        refuse "copying their analysis files failed"
    fi
    if [ -f "$SNAP/covered" ]; then
        mkdir -p "$INDEX_ROOT/js-environments-work"
        cut -f1 "$BEFORE" | sort -u - "$SNAP/covered" > "$INDEX_ROOT/js-environments-work/covered"
    fi
    COUNT=$(wc -l < "$TAKEN")
    if [ -z "$REMAINING" ]; then
        rm -rf "$SNAP"
    else
        echo "$REMAINING" > "$SNAP/consumers"
    fi
    echo "web-analysis-share: $TREE_NAME's web analyses are $PRODUCER's: $COUNT analysis files, in $((SECONDS - START)) s."
    if [ -s "$OWN" ]; then
        echo "web-analysis-share: but for $(wc -l < "$OWN") files $PRODUCER had other analyses of before its steps, which its js-analyze step analyzes (ex: $(examples "$OWN"))."
    fi
    if [ -s "$KEPT" ]; then
        echo "web-analysis-share: and it keeps its own analyses of $(wc -l < "$KEPT") files $PRODUCER had none of before its steps (ex: $(examples "$KEPT"))."
    fi
    rm -f "$BEFORE" "$KEPT" "$TAKEN"
    ;;
*)
    echo "Usage: web-analysis-share.sh list|snapshot|restore CONFIG_FILE TREE_NAME [FAILED|PRODUCER]" 1>&2
    exit 1
    ;;
esac
