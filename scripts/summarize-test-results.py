#!/usr/bin/env python3

# Summarize the per-test CI results that https://tests.firefox.dev/ shows into
# a file that per-file-info.toml can ingest.
#
# The inputs are the "<harness>-issues.json" artifacts of the
# source-test-info-<harness>-timings Taskcluster tasks, which cover every test
# that ran on trunk in the last 21 days.  The stats are computed the same way
# as on https://tests.firefox.dev/test.html (see computeTestStats in
# lib/query/test-stats.ts of https://github.com/mozilla/aretestsfastyet).
#
# Usage: summarize-test-results.py <output.json> <harness>-issues.json...

import json
import math
import os
import sys
from collections import Counter

# How many distinct failure messages and crash signatures to keep for each test.
MAX_ISSUES = 5


def harness_of(filename):
    return os.path.basename(filename).split("-")[0]


def status_kind(status):
    base = status.removesuffix("-PARALLEL").removesuffix("-SEQUENTIAL")
    return {
        "PASS": "pass",
        "OK": "pass",
        "FAIL": "fail",
        "TIMEOUT": "timeout",
        "CRASH": "crash",
        "SKIP": "skip",
        "EXPECTED-FAIL": "expected-fail",
    }.get(base, "unknown")


def format_pass_rate(passes, runs):
    # Math.round(ratio * 10000) / 100, as on tests.firefox.dev.
    rate = math.floor(passes / runs * 10000 + 0.5) / 100
    return int(rate) if rate == int(rate) else rate


def top_issues(counter, key):
    return [
        {key: value, "count": count}
        for value, count in counter.most_common(MAX_ISSUES)
    ]


def summarize(data, harness):
    tables = data["tables"]
    statuses = tables["statuses"]
    test_info = data["testInfo"]
    for test_id, groups in enumerate(data["testRuns"]):
        directory = tables["testPaths"][test_info["testPathIds"][test_id]]
        name = tables["testNames"][test_info["testNameIds"][test_id]]
        counts = Counter()
        failures = Counter()
        crashes = Counter()
        for status_id, group in enumerate(groups or []):
            if not group:
                continue
            kind = status_kind(statuses[status_id])
            for i, count in enumerate(group["counts"]):
                message = None
                if "messageIds" in group and group["messageIds"][i] is not None:
                    message = tables["messages"][group["messageIds"][i]]
                if kind == "skip" and message and message.startswith("run-if"):
                    # The test not running on other platforms is expected.
                    continue
                counts[kind] += count
                if kind == "fail" and message:
                    failures[message] += count
                elif kind == "crash" and "crashSignatureIds" in group:
                    signature_id = group["crashSignatureIds"][i]
                    if signature_id is not None:
                        crashes[tables["crashSignatures"][signature_id]] += count

        runs = sum(counts[kind] for kind in
                   ("pass", "fail", "timeout", "crash", "expected-fail"))
        summary = {
            "path": f"{directory}/{name}" if directory else name,
            "harness": harness,
            "runs": runs,
            "failures": counts["fail"],
            "timeouts": counts["timeout"],
            "crashes": counts["crash"],
            "skips": counts["skip"],
        }
        if runs:
            summary["pass_rate"] = format_pass_rate(
                counts["pass"] + counts["expected-fail"], runs)
        if failures:
            summary["top_failures"] = top_issues(failures, "message")
        if crashes:
            summary["top_crashes"] = top_issues(crashes, "signature")
        yield summary


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <output.json> <harness>-issues.json...",
              file=sys.stderr)
        sys.exit(1)

    output = {"tests": {}}
    for filename in sys.argv[2:]:
        if not os.path.exists(filename):
            print(f"Skipping missing {filename}", file=sys.stderr)
            continue
        with open(filename) as f:
            data = json.load(f)
        harness = harness_of(filename)
        metadata = data["metadata"]
        output["tests"][harness] = [
            dict(summary,
                 start_date=metadata["startDate"],
                 end_date=metadata["endDate"])
            for summary in summarize(data, harness)
        ]

    with open(sys.argv[1], "w") as f:
        json.dump(output, f)


if __name__ == "__main__":
    main()
