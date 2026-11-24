#!/usr/bin/env python3

# Carry the try pushes which the channel's web server imported (see
# tools/src/try_import.rs) over to the index being built, for its web server to
# import them again, since imports are only on the old web server's index
# volume, on the index's file system INDEX_FS (which the new web server mounts
# at /index):
# - For each tree with a sync-spec-prs hook in the config repository, the
#   spec pull requests it imported (its `spec-prs-imported` data API route), as
#   `{"pushes": [...]}` in its history_path's try-imports/restore.json (see
#   tools/src/spec_prs.rs's `start`).
# - For each other tree with an import-try hook, its inventory of its imported
#   try pushes (its `try-imports/inventory` route, only for the indexer, not
#   through the load balancer), as its history_path's
#   try-imports/TREE.carried-over.json, whose live set the new web server
#   imports again (see tools/src/try_carry_over.rs), as cutover-gate.py waits
#   for before the new web server takes over.
# The web server is the channel's target group's (see trigger-web-server.py),
# or the one at --from's base URL (ex: http://localhost:16995, for trying it).
# At best effort: what goes wrong is a warning (ex: before the web server has
# the routes).
#
# Usage: carry-over-imports.py CHANNEL CONFIG_REPO CONFIG_JSON INDEX_FS
#            [--from BASE_URL]

import json
import os
import sys
import urllib.request

USER_AGENT = "searchfox-indexer (carry-over-imports.py)"


def warn(message):
    print(f"WARNING: carry-over-imports: {message}", flush=True)


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


def spec_pr_trees(config, config_repo):
    """The trees which import spec pull requests (with a sync-spec-prs hook)."""
    return [name for name in config.get("trees", {})
            if os.path.exists(os.path.join(config_repo, name, "sync-spec-prs"))]


def try_push_trees(config, config_repo):
    """The other trees which import try pushes (with an import-try hook; see
    tools/src/try_import.rs's `hook`)."""
    spec_trees = set(spec_pr_trees(config, config_repo))
    return [name for name in config.get("trees", {})
            if name not in spec_trees
            and os.access(os.path.join(config_repo, name, "import-try"), os.X_OK)
            and config["trees"][name].get("history_path")]


def fetch_json(servers, tree, route):
    """The JSON of the first of the servers answering the tree's route."""
    for server in servers:
        url = f"{server}/{tree}/{route}"
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=120) as response:
                return json.loads(response.read())
        except Exception as e:
            warn(f"{tree}: {url}: {e}")
    return None


def write_carried(config, tree, index_fs, name, value):
    """Write `value` as JSON to the tree's history_path's try-imports/NAME, in
    the new index (where the web server sees it, under /index)."""
    path = os.path.join(config["trees"][tree]["history_path"], "try-imports", name)
    if path.startswith("/index/"):
        path = os.path.join(index_fs, path[len("/index/"):])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(value, f, indent=1)
        f.write("\n")
    return path


def main():
    args = sys.argv[1:]
    base = None
    if "--from" in args:
        at = args.index("--from")
        base = args[at + 1]
        del args[at:at + 2]
    channel, config_repo, config_json, index_fs = args
    with open(config_json) as f:
        config = json.load(f)
    spec_trees = [tree for tree in spec_pr_trees(config, config_repo)
                  if config["trees"][tree].get("history_path")]
    push_trees = try_push_trees(config, config_repo)
    if not spec_trees and not push_trees:
        return
    try:
        servers = [base] if base else channel_servers(channel)
    except Exception as e:
        warn(f"the channel's web server couldn't be found: {e}")
        return
    if not servers:
        warn("the channel has no web server")
        return
    for tree in spec_trees:
        pushes = fetch_json(servers, tree, "api/v1/spec-prs-imported")
        if not pushes:
            print(f"{tree}: no imported spec pull requests to carry over", flush=True)
            continue
        path = write_carried(config, tree, index_fs, "restore.json", {"pushes": pushes})
        print(f"{tree}: carried {len(pushes)} imported spec pull requests over, to {path}",
              flush=True)
    for tree in push_trees:
        inventory = fetch_json(servers, tree, "try-imports/inventory")
        pushes = (inventory or {}).get("pushes") or []
        if not pushes:
            print(f"{tree}: no imported try pushes to carry over", flush=True)
            continue
        path = write_carried(config, tree, index_fs, f"{tree}.carried-over.json",
                             {"pushes": pushes})
        print(f"{tree}: carried {len(pushes)} imported try pushes over, to {path}",
              flush=True)


try:
    main()
except Exception as e:
    warn(str(e))
