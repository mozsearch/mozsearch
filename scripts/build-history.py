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
import collections
import os
import re
import shlex
import subprocess
import sys
import threading
import time


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


def format_duration(seconds):
    minutes = int(seconds // 60)
    days, minutes = divmod(minutes, 24 * 60)
    hours, minutes = divmod(minutes, 60)
    if days:
        return f"{days}d {hours}h"
    if hours:
        return f"{hours}h {minutes}m"
    return f"{minutes}m"


class Status:
    """Reports status messages to the log and to the --status-command (with
    the message as its last argument), at most once a minute unless forced."""

    INTERVAL = 60

    def __init__(self, command):
        self.command = shlex.split(command) if command else None
        self.last = None
        # Reports come from the thread reading a tool's log and its heartbeat.
        self.lock = threading.Lock()

    def report(self, message, force=False):
        with self.lock:
            now = time.monotonic()
            if not force and self.last is not None and now - self.last < self.INTERVAL:
                return
            self.last = now
            log(message)
            if self.command:
                try:
                    subprocess.run(self.command + [message], timeout=120)
                except (OSError, subprocess.SubprocessError) as e:
                    log(f"The status command failed: {e}")


class Progress:
    """A tool's progress through its revisions across chunks, which we get
    from its log: it says how many revisions it has to process when it starts
    and logs "progress I/N" as it starts each one.  (Only for reporting; if the
    log changes, we just report less.)"""

    REMAINING = re.compile(rb"(\d+) revisions to process")
    STARTED = re.compile(rb"progress (\d+)/\d+")

    # Heartbeat reports mention the current revision once it has taken this
    # long (ex: a history window's start, which takes minutes).
    SLOW_REVISION = 120
    # The rate (and so the ETA) is over about this long, so that a slow
    # revision (ex: that window start) doesn't skew it for long.
    RATE_WINDOW = 600

    def __init__(self, label, status):
        self.label = label
        self.status = status
        self.start = time.monotonic()
        # The revisions processed before the current chunk.
        self.processed = 0
        self.done = 0
        self.total = None
        # When the current revision started, and whether we've reported yet.
        self.revision_started = None
        self.reported = False
        # (time, done) samples for the rate, oldest first.
        self.samples = collections.deque([(self.start, 0)])

    def chunk_started(self, processed):
        self.processed = self.done = processed
        self.revision_started = None

    def log_line(self, line):
        if b"revisions to process" in line and (m := self.REMAINING.search(line)):
            self.total = self.processed + int(m.group(1))
        elif b"progress " in line and (m := self.STARTED.search(line)):
            self.done = self.processed + int(m.group(1)) - 1
            self.revision_started = time.monotonic()
            self.samples.append((self.revision_started, self.done))
            # Keep one sample from before the window.
            while len(self.samples) > 2 and self.samples[1][0] < self.revision_started - self.RATE_WINDOW:
                self.samples.popleft()
            # Always report the tool starting, even just after the last tool's
            # final report.  (Later chunks start just after a report.)
            self.report(force=not self.reported)
            self.reported = True

    def report(self, force=False):
        now = time.monotonic()
        message = f"{self.label}: {self.done}"
        if self.total:
            message += f"/{self.total} revisions ({100 * self.done / self.total:.1f}%)"
        else:
            message += " revisions"
        since, done_then = self.samples[0]
        if self.done > done_then and now > since:
            per_second = (self.done - done_then) / (now - since)
            message += f", {60 * per_second:.0f}/min"
            if self.total:
                message += f", ETA {format_duration((self.total - self.done) / per_second)}"
        started = self.revision_started
        if started is not None and now - started >= self.SLOW_REVISION:
            message += f", on revision {self.done + 1} for {format_duration(now - started)}"
        self.status.report(message, force)


def heartbeat(progress, stop):
    """Report the progress every minute (subject to Status's limit) even when
    the tool logs nothing, so that a long revision doesn't look like a hang."""
    while not stop.wait(Status.INTERVAL):
        progress.report()


def run_tool(command, limit, progress):
    """Run a tool with COMMIT_LIMIT, passing its log through."""
    proc = subprocess.Popen(
        command, env=dict(os.environ, COMMIT_LIMIT=str(limit)), stderr=subprocess.PIPE
    )
    stop = threading.Event()
    beat = threading.Thread(target=heartbeat, args=(progress, stop), daemon=True)
    beat.start()
    try:
        for line in proc.stderr:
            sys.stderr.buffer.write(line)
            progress.log_line(line)
        sys.stderr.buffer.flush()
    finally:
        stop.set()
        beat.join()
    if proc.wait() != 0:
        raise subprocess.CalledProcessError(proc.returncode, command)


def run_in_chunks(name, label, command, repo, ref, chunk_size, total_limit, status):
    """Run `command` with COMMIT_LIMIT until it has processed every revision
    (or `total_limit` revisions, if nonzero), repacking `repo` after each run.
    """
    progress = Progress(label, status)
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
        progress.chunk_started(processed)
        run_tool(command, limit, progress)
        after = num_processed(repo, ref)
        log(f"{name} processed {after - before} revisions; repacking {repo}")
        repack(repo)
        processed += after - before
        progress.chunk_started(processed)
        progress.report(force=True)
        # The tools only process fewer revisions than the limit if that's all
        # there were.
        if after - before < limit:
            break
        before = after
    status.report(
        f"{label}: processed {processed} revisions in {chunk} chunks in "
        f"{format_duration(time.monotonic() - progress.start)}",
        force=True,
    )


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
    parser.add_argument(
        "--status-command",
        help="A command to run with a progress message (ex: \"timeline: "
        "1200/5000 revisions (24.0%%), 350/min, ETA 11m\") as its last argument, "
        "at most once a minute (ex: infrastructure/aws/set-status.py).",
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
    status = Status(args.status_command)
    run_in_chunks(
        "build-syntax-token-tree",
        "syntax",
        [tool("build-syntax-token-tree"), args.source_repo, syntax, *args.syntax_args],
        syntax,
        ref,
        args.chunk_size,
        total_limit,
        status,
    )
    run_in_chunks(
        "build-timeline-tree",
        "timeline",
        [tool("build-timeline-tree"), args.source_repo, syntax, timeline, rev_summaries],
        timeline,
        ref,
        args.chunk_size,
        total_limit,
        status,
    )


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        log(f"{' '.join(map(str, e.cmd))} failed with exit code {e.returncode}")
        sys.exit(1)
