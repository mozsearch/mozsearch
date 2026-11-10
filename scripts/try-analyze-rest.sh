#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's analyses by the rest of indexing's steps after its web
# analyses (see mkindex.sh), as they make the tree's, for its semantic delta
# (see tools/src/try_delta.rs), into OUT_DIR (its analysis by path, ex: its
# compilation's; see build-try-delta): of the files in CHANGED (a list of
# paths, ex: those the push changes, and `__GENERATED__/` ones, the generated
# files its build changes) at FILES_ROOT (its head's checkout), with its
# build's OBJDIR (its generated files).  The analyzers reading other files'
# analysis (ex: generated headers' C++) read OUT_DIR's, and the tree's
# (INDEX_ROOT's analysis, gzipped or not) for the files OUT_DIR doesn't have,
# and both for those it has only some of the records of (INCOMPLETE, a list
# of them, as build-try-delta's --incomplete).
# 1. Its XPIDL's: the `.idl` files it changes, or whose headers its build
#    changes.
# 2. Its WebIDL's, if it changes a `.webidl` file or a binding's C++: all of
#    the tree's WebIDL at its head, since a file's records can be another's
#    (ex: a mixin's, of the interfaces including it); build-try-delta keeps
#    those whose records the push changes.
# 3. Its IPDL's: the `.ipdl` and `.ipdlh` files it changes, or whose
#    generated C++ its build changes (ex: for a change to an `.ipdlh` file
#    they include), with the files they include.
# 4. Its TOML's (ex: test manifests').
# 5. Its semantic analysis (see semantic-analyze), written for OUT_DIR's files
#    (whose analysis it adds to, ex: C++ files' uses of prefs) and those it
#    changes, with what all of the tree's files at its head define, for what
#    their uses can name (those files' own, and the tree's others', which its
#    indexing saved).
# 6. Its moz.build files', with the tree's generated files and OBJDIR's.
# 7. And all of OUT_DIR's files' URLs (replace-aliases.py, with the tree's URL
#    map) and functions' GC (annotate-gc, with the tree's hazard analysis), as
#    indexing's last steps add them to the tree's, so that the files whose
#    records the push doesn't change have the tree's.
# A file's kind is by the tree's lists (INDEX_ROOT's idl-files and the like,
# which its repo_files filters made), or its extension, for a file the push
# adds.  Since its steps add to the analysis of OUT_DIR's files, this is the
# last of a push's analyses.  Each step says when it starts, and a step that
# fails says so, without stopping the others.
#
# Usage: try-analyze-rest.sh CONFIG_FILE TREE_NAME INDEX_ROOT FILES_ROOT OBJDIR CHANGED OUT_DIR [INCOMPLETE]

if [ $# -ne 7 ] && [ $# -ne 8 ]; then
    echo "Usage: try-analyze-rest.sh CONFIG_FILE TREE_NAME INDEX_ROOT FILES_ROOT OBJDIR CHANGED OUT_DIR [INCOMPLETE]" 1>&2
    exit 1
fi
CONFIG_FILE=$(realpath "$1")
TREE_NAME=$2
INDEX_ROOT=$(realpath "$3")
FILES_ROOT=$(realpath "$4")
OBJDIR=$(realpath "$5")
CHANGED=$(realpath "$6")
OUT_DIR=$(realpath "$7")
SCRIPTS=$(cd "$(dirname "$0")" && pwd)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
if [ $# -eq 8 ] && [ -f "$8" ]; then
    cp "$8" "$WORK/incomplete"
else
    : > "$WORK/incomplete"
fi
OVERLAY=(--base-analysis "$INDEX_ROOT/analysis" --incomplete "$WORK/incomplete")
tree_setting() {
    jq -r --arg tree "$TREE_NAME" ".trees[\$tree].$1 // empty" "$CONFIG_FILE"
}
# A list of the tree's (ex: idl-files), if it has it.
tree_list() {
    if [ -f "$INDEX_ROOT/$1" ]; then cat "$INDEX_ROOT/$1"; fi
}

# (The push's files at its head; those it removes; the generated files its
# build changes; and the tree's files, and generated files, at its head.)
{ grep -v '^__GENERATED__/' "$CHANGED" || true; } | while IFS= read -r FILE; do
    if [ -e "$FILES_ROOT/$FILE" ]; then echo "$FILE"; fi
done | LC_ALL=C sort -u > "$WORK/at-head"
{ grep -v '^__GENERATED__/' "$CHANGED" || true; } | while IFS= read -r FILE; do
    if [ ! -e "$FILES_ROOT/$FILE" ]; then echo "$FILE"; fi
done | LC_ALL=C sort -u > "$WORK/removed"
{ grep '^__GENERATED__/' "$CHANGED" || true; } | LC_ALL=C sort -u > "$WORK/generated"
{ tree_list repo-files; cat "$WORK/at-head"; } | LC_ALL=C sort -u |
    LC_ALL=C comm -23 - "$WORK/removed" > "$WORK/repo-files"
{ tree_list objdir-files; cat "$WORK/generated"; } | LC_ALL=C sort -u > "$WORK/objdir-files"
tree_list repo-files > "$WORK/tree-files"

# The push's files at its head of a kind: those the tree's list ($1, ex:
# idl-files) has, and those it adds with the kind's extensions ($2, an awk
# regexp, which awk gets in its environment, since its -v would take the
# regexp's backslashes as escapes).
of_kind() {
    tree_list "$1" > "$WORK/listed"
    PATTERN=$2 awk 'FILENAME == ARGV[1] { tree[$0] = 1; next }
         FILENAME == ARGV[2] { listed[$0] = 1; next }
         $0 in listed || (!($0 in tree) && $0 ~ ENVIRON["PATTERN"]) { print }' \
        "$WORK/tree-files" "$WORK/listed" "$WORK/at-head"
}
# All of the tree's files of a kind at the push's head (in the tree's list's
# order, then those it adds), its generated ones too.
all_of_kind() {
    { tree_list "$1"; of_kind "$1" "$2"; } |
        awk 'FILENAME == ARGV[1] { gone[$0] = 1; next } !($0 in gone) && !seen[$0]++' "$WORK/removed" -
}
# The files of the list $1 which the generated files of the list $2 are named
# for (ex: nsIFoo.idl, for `__GENERATED__/dist/include/nsIFoo.h`): by their
# names without their directories and extensions, of the generated files
# matching the awk regexp $3, also without the suffix $4, if it's given (an
# awk regexp, ex: `(Parent|Child)$`).
named_by_generated() {
    PATTERN=$3 SUFFIX=$4 awk '
        function stem(path) { sub(/.*\//, "", path); sub(/\.[^.]*$/, "", path); return path }
        FILENAME == ARGV[1] { named[stem($0)] = $0; next }
        $0 ~ ENVIRON["PATTERN"] {
            name = stem($0)
            if (name in named) { print named[name] }
            else if (ENVIRON["SUFFIX"] != "" && sub(ENVIRON["SUFFIX"], "", name) && (name in named)) {
                print named[name]
            }
        }' "$1" "$2"
}

source "$SCRIPTS/idl-parsers.sh"
use_idl_parsers "$FILES_ROOT" "$WORK/pymodules"
IDL_ARGS=()
WEBIDL_ARGS=()
if [ "$(tree_setting do_not_fill_idl_binding_slots)" = "true" ]; then
    IDL_ARGS=(--do-not-fill-binding-slots)
    WEBIDL_ARGS=(--do-not-fill-js-binding-slots)
fi

echo "Performing try-analyze-rest::idl step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
all_of_kind idl-files '\.idl$' > "$WORK/all-idl"
{ of_kind idl-files '\.idl$'
  named_by_generated "$WORK/all-idl" "$WORK/generated" '^__GENERATED__/dist/include/[^/]*\.h$' ""
} | LC_ALL=C sort -u > "$WORK/idl"
echo "Analyzing the push's $(wc -l < "$WORK/idl") XPIDL files."
while IFS= read -r FILE; do
    mkdir -p "$(dirname "$OUT_DIR/$FILE")"
    "$SCRIPTS/idl-analyze.py" "$INDEX_ROOT" "$FILES_ROOT/$FILE" "$OBJDIR" "${IDL_ARGS[@]}" \
        --analysis-root "$OUT_DIR" "${OVERLAY[@]}" > "$OUT_DIR/$FILE" ||
        echo "WARNING: The push's $FILE couldn't be analyzed."
done < "$WORK/idl"

echo "Performing try-analyze-rest::webidl step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
of_kind webidl-files '\.webidl$' > "$WORK/webidl"
if [ -s "$WORK/webidl" ] ||
    grep -qE '^__GENERATED__/(dist/include/mozilla/dom/[^/]*Binding\.h|dom/bindings/[^/]*Binding\.cpp|.*\.webidl)$' "$WORK/generated"; then
    # (Its generated WebIDL is its build's.)
    mkdir -p "$WORK/webidl-index" "$WORK/webidl-cache"
    ln -s "$OBJDIR" "$WORK/webidl-index/objdir"
    all_of_kind webidl-files '\.webidl$' > "$WORK/all-webidl"
    echo "Analyzing the push's $(wc -l < "$WORK/webidl") WebIDL files, with the tree's other $(( $(wc -l < "$WORK/all-webidl") - $(wc -l < "$WORK/webidl") ))."
    BINDINGS=$(tree_setting webidl_binding_local_path)
    "$SCRIPTS/webidl-analyze.py" "$WORK/webidl-index" "$FILES_ROOT" "$OUT_DIR" "$WORK/webidl-cache" \
        "${BINDINGS:-null}" "${WEBIDL_ARGS[@]}" "${OVERLAY[@]}" < "$WORK/all-webidl" ||
        echo "WARNING: The push's WebIDL couldn't be analyzed."
fi

echo "Performing try-analyze-rest::ipdl step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
all_of_kind ipdl-files '\.ipdlh?$' > "$WORK/all-ipdl"
{ of_kind ipdl-files '\.ipdlh?$'
  named_by_generated "$WORK/all-ipdl" "$WORK/generated" '^__GENERATED__/' '(Parent|Child)$'
} | LC_ALL=C sort -u > "$WORK/ipdl"
echo "Analyzing the push's $(wc -l < "$WORK/ipdl") IPDL files."
if [ -s "$WORK/ipdl" ]; then
    # (The tree's include directories, and those of the files it adds.)
    read -r -a INCLUDES <<< "$(tree_list ipdl-includes)"
    while IFS= read -r FILE; do
        INCLUDES+=(-I "$(dirname "$FILE")")
    done < "$WORK/ipdl"
    (cd "$FILES_ROOT" &&
        xargs -d '\n' ipdl-analyze "${INCLUDES[@]}" -f "$WORK/repo-files" -o "$WORK/objdir-files" \
            -b "$FILES_ROOT" -a "$OUT_DIR" "${OVERLAY[@]}" --only "$WORK/ipdl" < "$WORK/ipdl") ||
        echo "WARNING: The push's IPDL couldn't be analyzed."
fi

echo "Performing try-analyze-rest::toml step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
of_kind toml-files '\.toml$' > "$WORK/toml"
echo "Analyzing the push's $(wc -l < "$WORK/toml") TOML files."
"$SCRIPTS/toml-analyze.py" "$INDEX_ROOT" "$FILES_ROOT" "$OUT_DIR" < "$WORK/toml" ||
    echo "WARNING: The push's TOML couldn't be analyzed."

echo "Performing try-analyze-rest::semantic step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
{ (cd "$OUT_DIR" && find . -type f | sed 's#^\./##'); cat "$WORK/at-head"; } |
    LC_ALL=C sort -u > "$WORK/semantic-files"
# (With what the tree's other files define, if its indexing saved that, so
# that only these files are analyzed: on firefox at config8's 2026-11-09
# revision, all of its 13,131 files took 25 s.)
RESOLVABLES=()
if [ -f "$INDEX_ROOT/semantic-resolvables.jsonl" ]; then
    RESOLVABLES=(--resolvables "$INDEX_ROOT/semantic-resolvables.jsonl")
fi
semantic-analyze --write-only "$WORK/semantic-files" "${RESOLVABLES[@]}" "$FILES_ROOT" "$OUT_DIR" \
    "$INDEX_ROOT/doc-trees.json" < "$WORK/repo-files" ||
    echo "WARNING: The push's semantic analysis failed."

echo "Performing try-analyze-rest::mozbuild step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
of_kind mozbuild-files '\.(build|mozbuild)$' > "$WORK/mozbuild"
echo "Analyzing the push's $(wc -l < "$WORK/mozbuild") moz.build files."
"$SCRIPTS/mozbuild-analyze.py" "$INDEX_ROOT" "$FILES_ROOT" "$INDEX_ROOT/objdir" "$OUT_DIR" "$OBJDIR" \
    < "$WORK/mozbuild" ||
    echo "WARNING: The push's moz.build files couldn't be analyzed."

echo "Performing try-analyze-rest::replace-aliases step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
(cd "$OUT_DIR" && find . -type f | sed 's#^\./##' | LC_ALL=C sort) > "$WORK/out-files"
if [ -f "$INDEX_ROOT/aliases/url-map.json" ] && [ -s "$WORK/out-files" ]; then
    parallel --pipepart -a "$WORK/out-files" --block -8 --halt 2 \
        "$SCRIPTS/replace-aliases.py" "$OUT_DIR" "$INDEX_ROOT/aliases/url-map.json" ||
        echo "WARNING: The push's analysis' URLs couldn't be replaced."
fi

echo "Performing try-analyze-rest::annotate-gc step : $(date +"%Y-%m-%dT%H:%M:%S%z")"
if [ -f "$INDEX_ROOT/gcFunctions.txt" ] && [ -f "$INDEX_ROOT/allFunctions.txt" ]; then
    annotate-gc "$OUT_DIR" "$INDEX_ROOT/gcFunctions.txt" "$INDEX_ROOT/allFunctions.txt" \
        < "$WORK/out-files" ||
        echo "WARNING: The push's analysis' functions' GC couldn't be annotated."
fi
