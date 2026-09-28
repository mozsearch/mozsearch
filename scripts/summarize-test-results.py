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
from collections import Counter, defaultdict
from datetime import date

# The kinds of results counted per day, besides runs.
COUNTED_KINDS = ("fail", "timeout", "crash", "skip")
KIND_NAMES = {
    "fail": "failures",
    "timeout": "timeouts",
    "crash": "crashes",
    "skip": "skips",
}


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


def summarize(data, harness, day_offset, days):
    """Yield the summary of each test, with daily counts over `days` days
    starting `day_offset` days after the start of the data."""
    tables = data["tables"]
    statuses = tables["statuses"]
    test_info = data["testInfo"]
    for test_id, groups in enumerate(data["testRuns"]):
        directory = tables["testPaths"][test_info["testPathIds"][test_id]]
        name = tables["testNames"][test_info["testNameIds"][test_id]]
        counts = Counter()
        daily = defaultdict(lambda: [0] * days)
        for status_id, group in enumerate(groups or []):
            if not group:
                continue
            kind = status_kind(statuses[status_id])
            day = 0
            for i, count in enumerate(group["counts"]):
                day += group["days"][i]
                message = None
                if "messageIds" in group and group["messageIds"][i] is not None:
                    message = tables["messages"][group["messageIds"][i]]
                if kind == "skip" and message and message.startswith("run-if"):
                    # The test not running on other platforms is expected.
                    continue
                counts[kind] += count
                if kind in ("pass", "expected-fail", "fail", "timeout", "crash"):
                    daily["runs"][day_offset + day] += count
                if kind in COUNTED_KINDS:
                    daily[kind][day_offset + day] += count

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
            # All the kinds are always present, to keep templates simple.
            "daily": {
                KIND_NAMES.get(kind, kind): daily[kind]
                for kind in ("runs",) + COUNTED_KINDS
            },
        }
        if runs:
            summary["pass_rate"] = format_pass_rate(
                counts["pass"] + counts["expected-fail"], runs)
        yield summary


def rate(count, total, digits=1):
    return round(count / total * 100, digits) if total else 0


def visible_rate(count, total):
    """A percentage for sparklines, rounded, but not to 0 unless it is 0."""
    return max(rate(count, total, 2), 0.01) if count else 0


def add_issue_fields(summary, daily, days):
    """Add the number of issues (failures, timeouts and crashes), and the daily
    percentages of runs with issues and of scheduled runs that were skipped,
    for the sparklines of directory listings.  They are left out if always
    zero, to keep the concise per-file info small."""
    issues = [
        sum(daily[kind][d] for kind in ("failures", "timeouts", "crashes"))
        for d in range(days)
    ]
    runs = daily["runs"]
    skips = daily["skips"]
    summary["issues"] = sum(issues)
    if any(issues):
        summary["daily_issue_rates"] = [
            visible_rate(issues[d], runs[d]) for d in range(days)]
    if any(skips):
        summary["daily_skip_rates"] = [
            visible_rate(skips[d], runs[d] + skips[d]) for d in range(days)]
    return issues


def add_daily_rates(summary, days):
    """Add the daily percentages of runs that failed, timed out or crashed, and
    of scheduled runs that were skipped, as on tests.firefox.dev/test.html."""
    daily = summary["daily"]
    runs = daily["runs"]
    rates = {
        kind: [rate(daily[kind][d], runs[d]) for d in range(days)]
        for kind in ("failures", "timeouts", "crashes")
    }
    rates["skips"] = [
        rate(daily["skips"][d], runs[d] + daily["skips"][d]) for d in range(days)
    ]
    summary["daily_rates"] = rates
    summary["max_daily_issue_rate"] = max(
        rates["failures"][d] + rates["timeouts"][d] + rates["crashes"][d]
        for d in range(days))
    summary["max_daily_skip_rate"] = max(rates["skips"])


def state_of(summary):
    """The state of a test over the whole window, as on
    https://tests.firefox.dev/flaky.html: flaky beats skipped beats stable.  A
    single issue in the whole window is considered noise."""
    if summary["issues"] > 1:
        return "flaky"
    if summary["skips"]:
        return "skipped"
    if summary["runs"]:
        return "stable"
    return None


def summarize_directories(tests, days):
    """Aggregate the test summaries into summaries of each directory
    containing tests."""
    directories = {}
    for test in tests:
        directory = os.path.dirname(test["path"])
        while directory:
            if directory not in directories:
                directories[directory] = {
                    "path": directory,
                    "tests": 0,
                    "flaky_tests": 0,
                    "skipped_tests": 0,
                    "stable_tests": 0,
                    "runs": 0,
                    "failures": 0,
                    "timeouts": 0,
                    "crashes": 0,
                    "skips": 0,
                    "daily": {
                        kind: [0] * days
                        for kind in ("runs", "failures", "timeouts", "crashes",
                                     "skips")
                    },
                }
            summary = directories[directory]
            summary["tests"] += 1
            if "state" in test:
                summary[test["state"] + "_tests"] += 1
            for key in ("runs", "failures", "timeouts", "crashes", "skips"):
                summary[key] += test[key]
            for kind, totals in summary["daily"].items():
                for day, value in enumerate(test["daily"][kind]):
                    totals[day] += value
            directory = os.path.dirname(directory)

    for summary in directories.values():
        daily = summary.pop("daily")
        issues = add_issue_fields(summary, daily, days)
        if summary["issues"]:
            summary["max_daily_issues"] = max(issues)
        # Directories get their daily counts in the concise per-file info, to
        # be shown in directory listings, so name the field differently from
        # the per-test one, which goes in the detailed per-file info.  Skips
        # are too numerous to be charted with the other issues.
        summary["daily_totals"] = {
            kind: daily[kind] for kind in ("failures", "timeouts", "crashes")
        }
    return list(directories.values())


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <output.json> <harness>-issues.json...",
              file=sys.stderr)
        sys.exit(1)

    inputs = []
    for filename in sys.argv[2:]:
        if not os.path.exists(filename):
            print(f"Skipping missing {filename}", file=sys.stderr)
            continue
        with open(filename) as f:
            inputs.append((harness_of(filename), json.load(f)))

    output = {"tests": {}}
    if inputs:
        # Align the daily counts of all the inputs on the same days.
        start = min(date.fromisoformat(data["metadata"]["startDate"])
                    for _, data in inputs)
        end = max(date.fromisoformat(data["metadata"]["endDate"])
                  for _, data in inputs)
        days = (end - start).days + 1
        period = {
            "start_date": start.isoformat(),
            "end_date": end.isoformat(),
            "days": days,
        }

        all_tests = []
        for harness, data in inputs:
            day_offset = (
                date.fromisoformat(data["metadata"]["startDate"]) - start
            ).days
            tests = list(summarize(data, harness, day_offset, days))
            for test in tests:
                add_issue_fields(test, test["daily"], days)
                add_daily_rates(test, days)
                state = state_of(test)
                if state:
                    test["state"] = state
                test.update(period)
            output["tests"][harness] = tests
            all_tests += tests

        directories = summarize_directories(all_tests, days)
        for directory in directories:
            directory.update(period)
        output["tests"]["directories"] = directories

    with open(sys.argv[1], "w") as f:
        json.dump(output, f)


if __name__ == "__main__":
    main()
