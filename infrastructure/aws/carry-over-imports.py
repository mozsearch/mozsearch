#!/usr/bin/env python3

# Carry the spec pull requests which the channel's web server imported (see
# tools/src/try_import.rs) over to the index being built, for its web server to
# import them again (see tools/src/spec_prs.rs's `restore_imports`), since
# imports are only on the old web server's index volume: for each tree with a
# sync-spec-prs hook in the config repository, the pull requests it imported
# (its `spec-prs-imported` data API route), as `{"pushes": [...]}` in its
# history_path's try-imports/restore.json, on the index's file system INDEX_FS
# (which the new web server mounts at /index).  The web server is the channel's
# target group's (see trigger-web-server.py), or the one at --from's base URL
# (ex: http://localhost:16995, for trying it).  At best effort: what goes wrong
# is a warning (ex: before the web server has the route).
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
    trees = [name for name in config.get("trees", {})
             if os.path.exists(os.path.join(config_repo, name, "sync-spec-prs"))]
    if not trees:
        return
    try:
        servers = [base] if base else channel_servers(channel)
    except Exception as e:
        warn(f"the channel's web server couldn't be found: {e}")
        return
    if not servers:
        warn("the channel has no web server")
        return
    for tree in trees:
        history = config["trees"][tree].get("history_path")
        if not history:
            continue
        pushes = None
        for server in servers:
            url = f"{server}/{tree}/api/v1/spec-prs-imported"
            try:
                request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
                with urllib.request.urlopen(request, timeout=60) as response:
                    pushes = json.loads(response.read())
                break
            except Exception as e:
                warn(f"{tree}: {url}: {e}")
        if not pushes:
            print(f"{tree}: no imported spec pull requests to carry over", flush=True)
            continue
        # (Where the web server sees it, under /index.)
        path = os.path.join(history, "try-imports", "restore.json")
        if path.startswith("/index/"):
            path = os.path.join(index_fs, path[len("/index/"):])
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            json.dump({"pushes": pushes}, f, indent=1)
            f.write("\n")
        print(f"{tree}: carried {len(pushes)} imported spec pull requests over, to {path}",
              flush=True)


try:
    main()
except Exception as e:
    warn(str(e))
