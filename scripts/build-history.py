#!/usr/bin/env python3
"""Build or update the token-centric history ("hyperblame", bug 1517978) of a
source repo: run build-syntax-token-tree and build-timeline-tree in chunks of
revisions, keeping the history repos fast to read as they go.

The timeline runs alongside the syntax (unless --no-pipeline is passed): after
each syntax chunk, the timeline processes the revisions the syntax has
processed so far while the syntax goes on to its next chunk.  (The timeline
tool processes the syntax commits it hasn't processed from the syntax repo's
branch, which only moves when a syntax chunk finishes.)  Each tool is mostly
limited by a single git fast-import process, so this nearly halves the time,
at the cost of both tools' memory at once.

Each run of a tool leaves a pack in its repo, and build-timeline-tree also
leaves a pack per merge (it has git fast-import checkpoint so that it can read
the merge's parents from disk), so a single run over a long history (ex: all
of firefox-main, which has 28k merges) would leave tens of thousands of packs:
looking up an object means looking in each pack (on a firefox window, ~900
packs made the timeline 9% slower than one), and each pack is an open file
while it's being read.  So after each chunk, the repo is repacked in the
background while the next chunk runs, and during a chunk if it has too many
packs (see `PackMaintenance`).  Chunking also
bounds how much state each git fast-import process accumulates (ex: its table
of every file name it has seen, which makes loading trees slower as it grows).
The tools resume from their source mapping notes, so a chunk is just a run
with COMMIT_LIMIT.

HISTORY_ROOT must contain the syntax/ and timeline/ repos, whose branch is
BLAME_REF (as for the tools; HEAD if unset), and rev-summaries/.  Arguments
after HISTORY_ROOT are passed to build-syntax-token-tree (ex: the history
configuration directory and old revision options).  Other environment
variables (ex: CINNABAR, NOTES_REF) are passed to the tools, except that
COMMIT_LIMIT limits how many revisions each tool processes in total.

Sending this process SIGUSR1 restarts it in place (as the same process, so
whatever is waiting for it keeps waiting): the tools stop after the revision
they're processing (see the tools' `history_stop`), and once they and any
repacks have finished, the script runs itself again with the same arguments,
which resumes from the tools' notes without losing anything.  So a running
reblame can be upgraded by updating the checkout and the tools and sending
SIGUSR1 (see docs/aws.md).  With HISTORY_PAUSE_ON_FAILURE set, a failure (ex:
a tool crashing) makes it wait for SIGUSR1 to restart like that (ex: after
fixing the tool) rather than exiting.
"""

import argparse
import collections
import glob
import os
import re
import resource
import shlex
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback


def log(message):
    print(f"build-history: {message}", file=sys.stderr, flush=True)


# The exit status of a tool which stopped because we asked it to (the tools'
# `history_stop::STOPPED_EXIT_CODE`).
STOPPED_EXIT_CODE = 75


class Stopped(Exception):
    """A tool stopped, or we didn't start one, because a restart was requested
    (see `Restart`)."""


class Restart:
    """Restarting this script in place on SIGUSR1; see the module docs."""

    def __init__(self):
        # The file that makes the tools stop (their `HISTORY_STOP_FILE`), which
        # is per process so that concurrent runs don't stop each other.
        self.stop_file = os.path.join(tempfile.gettempdir(), f"build-history-stop-{os.getpid()}")
        self._remove_stop_file()
        os.environ["HISTORY_STOP_FILE"] = self.stop_file
        self.requested = threading.Event()
        signal.signal(signal.SIGUSR1, self._request)

    def _remove_stop_file(self):
        try:
            os.remove(self.stop_file)
        except FileNotFoundError:
            pass

    def _request(self, signum, frame):
        if not self.requested.is_set():
            log("restarting (as SIGUSR1 requested) once the tools have stopped")
        with open(self.stop_file, "w"):
            pass
        self.requested.set()

    def check(self):
        if self.requested.is_set():
            raise Stopped()

    def restart(self):
        log("restarting")
        self._remove_stop_file()
        sys.stdout.flush()
        sys.stderr.flush()
        os.execv(sys.executable, [sys.executable, *sys.argv])


# The `Restart` for this process, set by main.
RESTART = None


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


# The threads a background repack's delta search uses, leaving most of the
# CPUs to the tools.
REPACK_THREADS = 8
# How many packs a repo can have during a chunk before we repack it.
MAX_PACKS = 64


class PackMaintenance:
    """Repacks a history repo in the background after each chunk (see the module
    docs), and during a chunk when it has more than MAX_PACKS packs, unless its
    last repack is still running.  (In the full firefox reblame's 2010 merges,
    build-timeline-tree's checkpoints left 600 packs 25 minutes into a chunk,
    and its merge threads spent most of their time waiting for each other to
    look up objects in each of them.)  Repacking between chunks
    took 17-68% of each tool's time in the full firefox reblame on AWS, mostly
    single-threaded, even though on a firefox window it didn't make the tools
    any faster: it combined the packs and made them smaller (2.5x for the
    timeline, 15% for the syntax, since the tools have git fast-import store
    blobs whole), which matters for the full history's disk space.

    git and libgit2 cope with a repack deleting packs while they read the
    repo: the new pack is written first, and an object not found in the packs
    they know about makes them look for new packs.  (Not with a
    multi-pack-index, though, which would make even thousands of packs as fast
    as one: libgit2 then takes an object's pack from it, and fails to read the
    object if the pack was deleted before it opened it.  So we don't write
    one.)"""

    def __init__(self, repo):
        self.repo = repo
        # The running `git repack`, if any.
        self.repacking = None

    def _repack_finished(self, wait=False):
        """Whether no `git repack` is running (after waiting for it to finish
        if `wait`)."""
        if self.repacking is None:
            return True
        returncode = self.repacking.wait() if wait else self.repacking.poll()
        if returncode is None:
            return False
        self.repacking = None
        if returncode != 0:
            raise subprocess.CalledProcessError(returncode, f"git -C {self.repo} repack")
        return True

    def after_chunk(self):
        if not self._repack_finished():
            log(f"not repacking {self.repo}, since its last repack is still running")
            return
        self._repack()

    def during_chunk(self):
        """Called periodically while a tool runs (from another thread than
        `after_chunk`, but never at the same time)."""
        if self.repacking is not None:
            # Leave a failed repack for `after_chunk` or `finish` to raise.
            if self.repacking.poll() != 0:
                return
            self.repacking = None
        packs = len(glob.glob(os.path.join(self.repo, ".git", "objects", "pack", "*.pack")))
        if packs > MAX_PACKS:
            log(f"repacking {self.repo}, which has {packs} packs")
            self._repack()

    def _repack(self):
        # Geometric repacking combines the small packs without rewriting the
        # big ones every time, keeping the number of packs logarithmic in the
        # number of objects.
        self.repacking = subprocess.Popen(
            [
                "git",
                "-C",
                self.repo,
                "repack",
                "-d",
                "-q",
                "--geometric=2",
                f"--threads={REPACK_THREADS}",
            ]
        )

    def finish(self):
        self._repack_finished(wait=True)


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
    the latest message of each tool, joined, as its last argument), at most
    once a minute unless forced."""

    INTERVAL = 60

    def __init__(self, command):
        self.command = shlex.split(command) if command else None
        self.last = None
        # The latest message of each tool, in the order they started.
        self.messages = {}
        # Reports come from the threads reading the tools' logs and their
        # heartbeats.
        self.lock = threading.Lock()

    def report(self, message, force=False, key=None):
        with self.lock:
            self.messages[key] = message
            now = time.monotonic()
            if not force and self.last is not None and now - self.last < self.INTERVAL:
                return
            self.last = now
            log(message)
            if self.command:
                try:
                    subprocess.run(self.command + ["; ".join(self.messages.values())], timeout=120)
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
        # Another tool's Progress whose total is at least ours, if any: when the
        # timeline follows the syntax, each timeline chunk only knows about the
        # revisions the syntax has processed so far.
        self.total_from = None

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
        total = self.total
        if self.total_from and self.total_from.total:
            total = max(total or 0, self.total_from.total)
        message = f"{self.label}: {self.done}"
        if total:
            message += f"/{total} revisions ({100 * self.done / total:.1f}%)"
        else:
            message += " revisions"
        since, done_then = self.samples[0]
        if self.done > done_then and now > since:
            per_second = (self.done - done_then) / (now - since)
            message += f", {60 * per_second:.0f}/min"
            if total:
                message += f", ETA {format_duration((total - self.done) / per_second)}"
        started = self.revision_started
        if started is not None and now - started >= self.SLOW_REVISION:
            message += f", on revision {self.done + 1} for {format_duration(now - started)}"
        self.status.report(message, force, key=self.label)


def heartbeat(progress, stop, during):
    """Report the progress every minute (subject to Status's limit) even when
    the tool logs nothing, so that a long revision doesn't look like a hang,
    and call `during`."""
    while not stop.wait(Status.INTERVAL):
        progress.report()
        during()


# The tools' processes, so that a failure of one can stop the other.
RUNNING = set()
RUNNING_LOCK = threading.Lock()


def terminate_running():
    with RUNNING_LOCK:
        for proc in RUNNING:
            proc.terminate()


def run_tool(command, limit, progress, during=lambda: None):
    """Run a tool with COMMIT_LIMIT, passing its log through, and calling
    `during` every minute while it runs."""
    proc = subprocess.Popen(
        command, env=dict(os.environ, COMMIT_LIMIT=str(limit)), stderr=subprocess.PIPE
    )
    with RUNNING_LOCK:
        RUNNING.add(proc)
    stop = threading.Event()
    beat = threading.Thread(target=heartbeat, args=(progress, stop, during), daemon=True)
    beat.start()
    try:
        for line in proc.stderr:
            sys.stderr.buffer.write(line)
            progress.log_line(line)
        sys.stderr.buffer.flush()
    finally:
        stop.set()
        beat.join()
        returncode = proc.wait()
        with RUNNING_LOCK:
            RUNNING.discard(proc)
    if returncode == STOPPED_EXIT_CODE:
        raise Stopped()
    if returncode != 0:
        raise subprocess.CalledProcessError(returncode, command)


class ChunkedTool:
    """Runs a tool with COMMIT_LIMIT in chunks, maintaining `repo`'s packs
    after each run, and reports its progress."""

    def __init__(self, name, label, command, repo, ref, chunk_size, total_limit, status):
        self.name = name
        self.label = label
        self.command = command
        self.repo = repo
        self.ref = ref
        self.chunk_size = chunk_size
        # The most revisions to process in total, if nonzero.
        self.total_limit = total_limit
        self.status = status
        self.progress = Progress(label, status)
        self.packs = PackMaintenance(repo)
        self.processed = 0
        self.chunks = 0

    def catch_up(self, after_chunk=None, stopped=lambda: False):
        """Run chunks until the tool has processed every revision there is to
        process (or total_limit revisions), calling `after_chunk` after each
        chunk, and stopping early if `stopped()`."""
        before = num_processed(self.repo, self.ref)
        while not stopped():
            limit = self.chunk_size
            if self.total_limit:
                limit = min(limit, self.total_limit - self.processed)
                if limit <= 0:
                    return
            RESTART.check()
            self.chunks += 1
            log(
                f"{self.name} chunk {self.chunks} (up to {limit} revisions, "
                f"{self.processed} processed so far)"
            )
            self.progress.chunk_started(self.processed)
            run_tool(self.command, limit, self.progress, self.packs.during_chunk)
            after = num_processed(self.repo, self.ref)
            log(f"{self.name} processed {after - before} revisions")
            self.packs.after_chunk()
            self.processed += after - before
            self.progress.chunk_started(self.processed)
            self.progress.report(force=True)
            if after_chunk:
                after_chunk()
            # The tools only process fewer revisions than the limit if that's
            # all there were.
            if after - before < limit:
                return
            before = after

    def finish(self):
        self.packs.finish()
        self.status.report(
            f"{self.label}: processed {self.processed} revisions in {self.chunks} chunks in "
            f"{format_duration(time.monotonic() - self.progress.start)}",
            force=True,
            key=self.label,
        )


class Pipeline:
    """Lets the timeline follow the syntax's chunks (see the module docs)."""

    def __init__(self):
        self.cond = threading.Condition()
        self.syntax_chunks = 0
        self.syntax_done = False
        self.failed = False

    def syntax_chunk_done(self):
        with self.cond:
            self.syntax_chunks += 1
            self.cond.notify_all()

    def finish_syntax(self):
        with self.cond:
            self.syntax_done = True
            self.cond.notify_all()

    def fail(self):
        with self.cond:
            self.failed = True
            self.cond.notify_all()

    def wait_for_syntax(self, seen):
        """Wait for a syntax chunk after the first `seen`, the end of the
        syntax, or a failure, returning (chunks, done, failed)."""
        with self.cond:
            self.cond.wait_for(
                lambda: self.syntax_chunks > seen or self.syntax_done or self.failed
            )
            return self.syntax_chunks, self.syntax_done, self.failed


def run_pipelined(syntax, timeline):
    pipeline = Pipeline()
    errors = []

    def fail(e):
        errors.append(e)
        pipeline.fail()
        # A tool which stopped for a restart stops the other one too, since
        # it stops for the same reason.
        if not isinstance(e, Stopped):
            terminate_running()

    def follow_syntax():
        try:
            seen = 0
            while True:
                seen, syntax_done, failed = pipeline.wait_for_syntax(seen)
                if failed:
                    return
                timeline.catch_up(stopped=lambda: pipeline.failed)
                if syntax_done:
                    break
            if not pipeline.failed:
                timeline.finish()
        except BaseException as e:
            fail(e)

    thread = threading.Thread(target=follow_syntax)
    thread.start()
    try:
        syntax.catch_up(after_chunk=pipeline.syntax_chunk_done, stopped=lambda: pipeline.failed)
        if not pipeline.failed:
            syntax.finish()
        pipeline.finish_syntax()
    except BaseException as e:
        fail(e)
    thread.join()
    for e in errors:
        if not isinstance(e, Stopped):
            raise e
    if errors:
        raise errors[0]


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
        "--no-pipeline",
        dest="pipeline",
        action="store_false",
        help="Run the timeline after the syntax rather than alongside it.",
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

    # Each pack a tool reads is an open file, and the timeline's packs of a
    # chunk (one per merge) and of the chunks since the last repack finished
    # could be more than the usual soft limit of 1024.
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    wanted = hard if hard != resource.RLIM_INFINITY else 1 << 20
    if soft != resource.RLIM_INFINITY and soft < wanted:
        resource.setrlimit(resource.RLIMIT_NOFILE, (wanted, hard))

    def tool(name):
        return os.path.join(args.tools_dir, name) if args.tools_dir else name

    ref = os.environ.get("BLAME_REF", "HEAD")
    total_limit = int(os.environ.get("COMMIT_LIMIT") or 0)
    syntax = os.path.join(args.history_root, "syntax")
    timeline = os.path.join(args.history_root, "timeline")
    rev_summaries = os.path.join(args.history_root, "rev-summaries")
    status = Status(args.status_command)
    syntax_tool = ChunkedTool(
        "build-syntax-token-tree",
        "syntax",
        [tool("build-syntax-token-tree"), args.source_repo, syntax, *args.syntax_args],
        syntax,
        ref,
        args.chunk_size,
        total_limit,
        status,
    )
    timeline_tool = ChunkedTool(
        "build-timeline-tree",
        "timeline",
        [tool("build-timeline-tree"), args.source_repo, syntax, timeline, rev_summaries],
        timeline,
        ref,
        args.chunk_size,
        total_limit,
        status,
    )
    global RESTART
    RESTART = Restart()
    try:
        if args.pipeline:
            timeline_tool.progress.total_from = syntax_tool.progress
            run_pipelined(syntax_tool, timeline_tool)
        else:
            syntax_tool.catch_up()
            syntax_tool.finish()
            timeline_tool.catch_up()
            timeline_tool.finish()
        return
    except Stopped:
        pass
    except Exception as e:
        if not os.environ.get("HISTORY_PAUSE_ON_FAILURE"):
            raise
        traceback.print_exc()
        failure = e
        if isinstance(e, subprocess.CalledProcessError):
            failure = f"{' '.join(map(str, e.cmd[:1]))} failed with exit code {e.returncode}"
        status.report(
            f"paused after a failure ({failure}); send SIGUSR1 to build-history.py "
            f"(pid {os.getpid()}) to restart it",
            force=True,
            key="paused",
        )
        while not RESTART.requested.wait(60):
            pass
    for chunked in (syntax_tool, timeline_tool):
        try:
            chunked.packs.finish()
        except subprocess.CalledProcessError as e:
            log(f"a repack of {chunked.repo} failed with exit code {e.returncode}")
    RESTART.restart()


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        log(f"{' '.join(map(str, e.cmd))} failed with exit code {e.returncode}")
        sys.exit(1)
