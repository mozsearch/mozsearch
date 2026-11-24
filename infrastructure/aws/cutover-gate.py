#!/usr/bin/env python3

# Wait, before a new web server takes over its channel from the old one (see
# trigger-web-server.py), for the new one to import again the live set of the
# try pushes the old one imported (see tools/src/try_carry_over.rs), so that
# nobody's review in progress disappears for the length of its import: for
# each tree which imports try pushes but spec pull requests (with an
# import-try hook in the config repository, but no sync-spec-prs hook), it
# posts the old web servers' inventory (their `try-imports/inventory` route)
# to the new one's `try-imports/restore` route (which imports again what's in
# its live set and isn't imported, waiting or running, after the inventory
# the indexer carried over to it; see carry-over-imports.py), at first and
# every --refresh-minutes (5), for what the old one imported since, and polls
# its progress (`try-imports/restore-status`) every 30 seconds, until none of
# its live set's imports wait or run (imported or failed), or for at most
# --minutes (the config's `cutover_wait_minutes`, 90 if it has none; what's
# left then goes on after the switch), and then posts the old ones'
# inventory once more.  At best effort: what goes wrong is a warning (ex: an
# old web server without the routes), and it always exits with 0.  Those
# routes are only for requests from inside, not through the load balancer.
#
# Usage: cutover-gate.py CONFIG_REPO CONFIG_FILE NEW_BASE_URL [OLD_BASE_URL...]
#            [--minutes N] [--refresh-minutes N]
#
# (ex: cutover-gate.py /vagrant/tests config.json http://localhost:16995
# http://localhost:16995, for trying it, with one server as both.)

import json
import os
import sys
import time
import urllib.request

USER_AGENT = "searchfox-indexer (cutover-gate.py)"
DEFAULT_MINUTES = 90
POLL_SECONDS = 30


def log(message):
    print(f"cutover-gate: {message}", flush=True)


def warn(message):
    print(f"WARNING: cutover-gate: {message}", flush=True)


def try_push_trees(config, config_repo):
    """The trees which import try pushes but spec pull requests (see
    carry-over-imports.py)."""
    return [name for name, tree in config.get("trees", {}).items()
            if tree.get("history_path")
            and os.access(os.path.join(config_repo, name, "import-try"), os.X_OK)
            and not os.path.exists(os.path.join(config_repo, name, "sync-spec-prs"))]


def request_json(url, body=None, timeout=120):
    data = None if body is None else json.dumps(body).encode()
    headers = {"User-Agent": USER_AGENT}
    if data is not None:
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read())


def old_inventory(olds, tree):
    """The first old web server's inventory of the tree's imported pushes."""
    for old in olds:
        url = f"{old}/{tree}/try-imports/inventory"
        try:
            return request_json(url).get("pushes") or []
        except Exception as e:
            warn(f"{tree}: {url}: {e}")
    return None


def post_inventories(new, olds, trees):
    """Post the old web servers' inventories to the new one; the trees whose
    restores the new one has (whose routes answered)."""
    answered = []
    for tree in trees:
        pushes = old_inventory(olds, tree) if olds else []
        if pushes is None:
            # (Its restore of what the indexer carried over still counts.)
            pushes = []
        url = f"{new}/{tree}/try-imports/restore"
        try:
            status = request_json(url, {"pushes": pushes})
            answered.append(tree)
            log(f"{tree}: posted {len(pushes)} pushes of the old web server's: {summary(status)}")
        except Exception as e:
            warn(f"{tree}: {url}: {e}")
    return answered


def summary(status):
    return ", ".join(f"{key} {status.get(key, 0)}"
                     for key in ("live", "done", "failed", "running", "queued", "notImported"))


def main():
    args = sys.argv[1:]
    options = {"--minutes": None, "--refresh-minutes": 5.0}
    for option in options:
        if option in args:
            at = args.index(option)
            options[option] = float(args[at + 1])
            del args[at:at + 2]
    config_repo, config_file, new, *olds = args
    new = new.rstrip("/")
    olds = [old.rstrip("/") for old in olds]
    with open(os.path.join(config_repo, config_file)) as f:
        config = json.load(f)
    minutes = options["--minutes"]
    if minutes is None:
        minutes = float(config.get("cutover_wait_minutes", DEFAULT_MINUTES))
    trees = try_push_trees(config, config_repo)
    if not trees:
        return
    started = time.monotonic()
    deadline = started + minutes * 60
    refresh = options["--refresh-minutes"] * 60
    trees = post_inventories(new, olds, trees)
    posted = time.monotonic()
    last = {}
    while trees:
        waiting = {}
        for tree in list(trees):
            url = f"{new}/{tree}/try-imports/restore-status"
            try:
                status = request_json(url, timeout=60)
            except Exception as e:
                warn(f"{tree}: {url}: {e}")
                continue
            if summary(status) != last.get(tree):
                last[tree] = summary(status)
                log(f"{tree} after {(time.monotonic() - started) / 60:.1f} min: {last[tree]}")
            if status.get("running", 0) + status.get("queued", 0) > 0:
                waiting[tree] = status
        if not waiting:
            log(f"the live sets are imported (or failed), after "
                f"{(time.monotonic() - started) / 60:.1f} min")
            break
        if time.monotonic() >= deadline:
            for tree, status in waiting.items():
                left = [push["push"] for push in status.get("pushes", [])
                        if push.get("state") in ("running", "queued")]
                log(f"{tree}: the deadline ({minutes:g} min) passed with {len(left)} "
                    f"of its live set left, which go on after the switch: {' '.join(left)}")
            break
        time.sleep(POLL_SECONDS)
        if time.monotonic() - posted >= refresh:
            post_inventories(new, olds, trees)
            posted = time.monotonic()
    # (What the old ones imported since the last post.)
    if trees and olds:
        post_inventories(new, olds, trees)


try:
    main()
except Exception as e:
    warn(str(e))
