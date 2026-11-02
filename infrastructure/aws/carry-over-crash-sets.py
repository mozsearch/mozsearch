#!/usr/bin/env python3

# Carry the crash sets which the channel's web server has (see
# tools/src/crash_sets.rs) over to the index being built, since its store is
# only on the old web server's index volume: first the new store's cap (how
# many bytes its sets and crashes may take, which its web server refuses
# ingestions beyond; see `CrashSetStore::cap`), by the index's file system's
# free space (INDEX_FS, which the new web server mounts at /index): at most
# 5 GB, and half of what's free beyond the nginx cache's 20 GB (see index.sh);
# then 60% of that's worth of the old store's sets, the most recently used
# first, but penalized for their sizes (which the web server chooses; see
# `carry_over`, its `crash-sets/inventory` route), and their files (its
# `crash-sets/files` route, in batches), and when they were last used (their
# `touched/ID` files' times).  The web server is the channel's target group's
# (see carry-over-imports.py), or the one at --from's base URL (ex:
# http://localhost:16995, for trying it).  Those routes are only for requests
# from the indexer, not through the load balancer.  At best effort: what goes
# wrong is a warning (ex: before the web server has the routes), but the cap
# is written first.  (Sets which the old web server ingests after this are
# left behind.)
#
# Usage: carry-over-crash-sets.py CHANNEL CONFIG_JSON INDEX_FS [--from BASE_URL]

import base64
import json
import os
import re
import shutil
import sys
import urllib.request

USER_AGENT = "searchfox-indexer (carry-over-crash-sets.py)"
DEFAULT_CAP = 5_000_000_000
# (index.sh's check of the nginx cache's space.)
NGINX_CACHE_RESERVE = 20 * 1024 * 1024 * 1024
CARRY_OVER_SHARE = 0.6
BATCH = 400
FILE_PATTERN = re.compile(
    r"^(sets/[0-9a-f]{16}\.json"
    r"|crashes/([0-9a-f]{2})/\2[0-9a-f]{6}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.json(\.gz)?)$")


def warn(message):
    print(f"WARNING: carry-over-crash-sets: {message}", flush=True)


def channel_servers(channel):
    """The base URLs of the web servers in the channel's target group."""
    import boto3
    import aws_credentials  # noqa: F401 (Before using boto3; see there.)

    elb = boto3.client("elbv2")
    ec2 = boto3.client("ec2")
    groups = elb.describe_target_groups(Names=[f"{channel}-target"])["TargetGroups"]
    health = elb.describe_target_health(TargetGroupArn=groups[0]["TargetGroupArn"])
    ids = [target["Target"]["Id"] for target in health["TargetHealthDescriptions"]]
    if not ids:
        return []
    reservations = ec2.describe_instances(InstanceIds=ids)["Reservations"]
    return [f"http://{instance['PrivateIpAddress']}"
            for reservation in reservations for instance in reservation["Instances"]
            if instance.get("PrivateIpAddress")]


def request(url, body=None):
    headers = {"User-Agent": USER_AGENT}
    if body is not None:
        headers["Content-Type"] = "text/plain"
    req = urllib.request.Request(url, data=body, headers=headers)
    with urllib.request.urlopen(req, timeout=300) as response:
        return response.read()


def store_path(config, index_fs):
    """The crash sets' store's directory on INDEX_FS: the config's
    `crash_sets_path`, else `crash-sets/` beside the trees' index directories
    (see the tools' `LocalIndex::crash_sets`)."""
    path = config.get("crash_sets_path")
    if not path:
        trees = list(config.get("trees", {}).values())
        if not trees or not trees[0].get("index_path"):
            return None
        path = os.path.join(os.path.dirname(trees[0]["index_path"].rstrip("/")), "crash-sets")
    if path.startswith("/index/"):
        path = os.path.join(index_fs, path[len("/index/"):])
    return path


def write_atomically(path, contents):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    temporary = f"{path}.tmp{os.getpid()}"
    with open(temporary, "wb") as f:
        f.write(contents)
    os.replace(temporary, path)


def copy_files(url, store, budget, files):
    """Copies the old store's `files` (at its web server's `url`) to `store`,
    within `budget` bytes, returning how many bytes they took."""
    written = 0
    for start in range(0, len(files), BATCH):
        batch = set(files[start:start + BATCH])
        lines = request(url, "\n".join(files[start:start + BATCH]).encode()).splitlines()
        for line in lines:
            item = json.loads(line)
            if item["path"] not in batch:
                continue
            contents = base64.b64decode(item["data"])
            if written + len(contents) > budget:
                warn("the files went over the budget")
                return written
            write_atomically(os.path.join(store, item["path"]), contents)
            written += len(contents)
    return written


def main():
    args = sys.argv[1:]
    base = None
    if "--from" in args:
        at = args.index("--from")
        base = args[at + 1]
        del args[at:at + 2]
    channel, config_json, index_fs = args
    with open(config_json) as f:
        config = json.load(f)
    trees = list(config.get("trees", {}))
    store = store_path(config, index_fs)
    if not store:
        return
    os.makedirs(store, exist_ok=True)

    free = shutil.disk_usage(index_fs).free
    cap = int(min(DEFAULT_CAP, max(0, (free - NGINX_CACHE_RESERVE) // 2)))
    write_atomically(os.path.join(store, "cap"), f"{cap}\n".encode())
    budget = int(cap * CARRY_OVER_SHARE)
    print(f"crash sets: the store's cap is {cap} bytes ({free} free on {index_fs}); "
          f"carrying over at most {budget}", flush=True)
    if budget == 0:
        return

    try:
        servers = [base] if base else channel_servers(channel)
    except Exception as e:
        warn(f"the channel's web server couldn't be found: {e}")
        return
    if not servers:
        warn("the channel has no web server")
        return
    # (Any tree's routes: its trees share the store.)
    inventory = None
    for server in servers:
        for tree in trees:
            url = f"{server}/{tree}/crash-sets/inventory?bytes={budget}"
            try:
                inventory = json.loads(request(url))
                break
            except Exception as e:
                warn(f"{url}: {e}")
        if inventory is not None:
            break
    if not inventory or not inventory.get("sets"):
        print("crash sets: none to carry over", flush=True)
        return

    written = copy_files(f"{server}/{tree}/crash-sets/files", store, budget,
                         [path for path in inventory["files"] if FILE_PATTERN.match(path)])
    # (When they were last used, for the next carry-over; the sets whose
    # manifests made it.)
    carried = 0
    for carried_set in inventory["sets"]:
        set_id = carried_set["id"]
        if not re.fullmatch(r"[0-9a-f]{16}", set_id) or \
                not os.path.exists(os.path.join(store, "sets", f"{set_id}.json")):
            continue
        touched = os.path.join(store, "touched", set_id)
        write_atomically(touched, b"")
        os.utime(touched, (carried_set["touched"], carried_set["touched"]))
        carried += 1
    print(f"crash sets: carried {carried} of the old store's over ({written} bytes), to {store}",
          flush=True)


try:
    main()
except Exception as e:
    warn(str(e))
