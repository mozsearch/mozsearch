#!/usr/bin/env python3
"""Build or update the token-centric history ("hyperblame", bug 1517978) of a
source repo: run build-syntax-token-tree and then build-timeline-tree in chunks
of revisions, repacking the history repos after each chunk.

Each run of a tool leaves a pack in its repo, and build-timeline-tree also
leaves a pack per merge (it has git fast-import checkpoint so that it can read
the merge's parents from disk), so a single run over a long history (ex: all
of firefox-main, which has 28k merges) would leave tens of thousands of packs,
which makes reading the repo slow.  Chunking also bounds how much state each
git fast-import process accumulates (ex: its table of every file name it has
seen, which makes loading trees slower as it grows).  The tools resume from
their source mapping notes, so a chunk is just a run with COMMIT_LIMIT.

HISTORY_ROOT must contain the syntax/ and timeline/ repos, whose branch is
BLAME_REF (as for the tools; HEAD if unset), and rev-summaries/.  Arguments
after HISTORY_ROOT are passed to build-syntax-token-tree (ex: the history
configuration directory and old revision options).  Other environment
variables (ex: CINNABAR, NOTES_REF) are passed to the tools, except that
COMMIT_LIMIT limits how many revisions each tool processes in total.
"""

import argparse
import os
import subprocess
import sys


def log(message):
    print(f"build-history: {message}", file=sys.stderr, flush=True)


def git_output(repo, *args):
    """The output of a git command, or None if it failed."""
    result = subprocess.run(
        ["git", "-C", repo, *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True
    )
    return result.stdout if result.returncode == 0 else None


def notes_ref(repo, blame_ref):
    """The notes ref in which the tools record the revisions they've processed
    in `repo` (see tools/src/source_mapping.rs): NOTES_REF, or
    refs/notes/mozsearch-source-mapping-<BRANCH>."""
    if os.environ.get("NOTES_REF"):
        return os.environ["NOTES_REF"]
    target = blame_ref
    if blame_ref == "HEAD":
        target = (git_output(repo, "symbolic-ref", "-q", "HEAD") or "HEAD").strip()
    for prefix in ("refs/heads/", "refs/"):
        if target.startswith(prefix):
            target = target[len(prefix) :]
            break
    return f"refs/notes/mozsearch-source-mapping-{target}"


def num_processed(repo, blame_ref):
    """The number of revisions processed into `repo`, which each have a note.
    (The branch doesn't have them all, since it points at the last commit
    written, which can be on a side branch.)"""
    notes = git_output(repo, "ls-tree", "-r", "--name-only", notes_ref(repo, blame_ref))
    return len(notes.splitlines()) if notes else 0


def repack(repo):
    # Geometric repacking combines the small packs that runs add without
    # rewriting the big ones every time, keeping the number of packs
    # logarithmic in the number of objects.
    subprocess.run(["git", "-C", repo, "repack", "-d", "-q", "--geometric=2"], check=True)


def run_in_chunks(name, command, repo, ref, chunk_size, total_limit):
    """Run `command` with COMMIT_LIMIT until it has processed every revision
    (or `total_limit` revisions, if nonzero), repacking `repo` after each run.
    """
    before = num_processed(repo, ref)
    processed = 0
    chunk = 0
    while True:
        limit = chunk_size
        if total_limit:
            limit = min(limit, total_limit - processed)
            if limit <= 0:
                break
        chunk += 1
        log(f"{name} chunk {chunk} (up to {limit} revisions, {processed} processed so far)")
        subprocess.run(command, check=True, env=dict(os.environ, COMMIT_LIMIT=str(limit)))
        after = num_processed(repo, ref)
        log(f"{name} processed {after - before} revisions; repacking {repo}")
        repack(repo)
        processed += after - before
        # The tools only process fewer revisions than the limit if that's all
        # there were.
        if after - before < limit:
            break
        before = after
    log(f"{name} processed {processed} revisions in {chunk} chunks")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--chunk-size",
        type=int,
        default=10000,
        help="The most revisions to process per run of a tool (default: 10000).",
    )
    parser.add_argument(
        "--tools-dir",
        help="The directory with the tools (default: find them on PATH).",
    )
    parser.add_argument("source_repo")
    parser.add_argument("history_root")
    parser.add_argument("syntax_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.chunk_size <= 0:
        parser.error("--chunk-size must be positive")

    def tool(name):
        return os.path.join(args.tools_dir, name) if args.tools_dir else name

    ref = os.environ.get("BLAME_REF", "HEAD")
    total_limit = int(os.environ.get("COMMIT_LIMIT") or 0)
    syntax = os.path.join(args.history_root, "syntax")
    timeline = os.path.join(args.history_root, "timeline")
    rev_summaries = os.path.join(args.history_root, "rev-summaries")
    run_in_chunks(
        "build-syntax-token-tree",
        [tool("build-syntax-token-tree"), args.source_repo, syntax, *args.syntax_args],
        syntax,
        ref,
        args.chunk_size,
        total_limit,
    )
    run_in_chunks(
        "build-timeline-tree",
        [tool("build-timeline-tree"), args.source_repo, syntax, timeline, rev_summaries],
        timeline,
        ref,
        args.chunk_size,
        total_limit,
    )


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        log(f"{' '.join(map(str, e.cmd))} failed with exit code {e.returncode}")
        sys.exit(1)
