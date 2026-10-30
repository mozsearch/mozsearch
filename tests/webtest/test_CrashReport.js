"use strict";

// A crash report's crash view (the `crash-report:` query term's; see
// tools/src/crash_stack.rs's `crash_report_stack`): its crashing thread's
// stack, each frame's inlined calls as frames of their own, beside their code
// at the indexed revision, with their lines from the report (mapped from the
// build's revision) highlighted; with `crash-threads:all`, every thread's; and
// the DWIM rule sending crash-stats reports' URLs there.  The report is
// tests/crash-reports/'s fixture (see webtest-config.json's
// `crash_reports_url`), whose frames are in the searchfox tree's
// tools/src/blame.rs at an old revision, whose lines have moved since.
const ID = "5e4c0b1a-7e57-4c0d-8e5a-000000261030";
const REV = "50df75fc5253890347bfac70037bc7ad42316862";

function doc() {
  return frame.contentDocument;
}

// (The crashing thread's, the first stack.)
const frames = () => [...doc().querySelector(".crash-stack").querySelectorAll(":scope > .crash-frame")];
const crashText = section => [...section.querySelectorAll(".crash-line")].map(line => line.textContent).join("\n");
const where = section => section.querySelector(".crash-frame-where")?.textContent || "";

add_task(async function test_CrashReport() {
  await TestUtils.loadQuery("searchfox", `crash-report:${ID}`);
  const notice = doc().querySelector(".crash-notice").textContent;
  ok(notice.includes(`The crash report ${ID} (Searchfox 1.0a1 nightly, build 20261030000000, on Linux)`),
     "The view says which report");
  ok(notice.includes("SIGSEGV / SEGV_MAPERR at 0x0, in thread #0 (Main Thread) of its 2 threads"),
     "what crashed, and where");
  is(doc().querySelector(".crash-reason code")?.textContent, "MOZ_RELEASE_ASSERT(phab_rev.is_some())",
     "and its MOZ_CRASH reason");

  const [strip, find, impl, patch, info, deserialize, libc] = frames();
  is(frames().length, 7, "Its 5 frames are 7, with the 2 calls inlined in them");
  ok(strip.classList.contains("crash-frame-inlined") && strip.querySelector(".crash-frame-inline"),
     "An inlined call is a frame of its own");
  ok(strip.querySelector(".crash-code-missing").textContent.startsWith("Not this tree's code"),
     "which isn't the tree's code if its file isn't (Rust's standard library's)");

  ok(crashText(find).includes("line.strip_prefix(PREFIX)"), "A frame's line from the report is highlighted");
  is(where(find), "At line 12 (at the build's revision, as now).", "as it was");
  is(find.querySelector(".crash-frame-build").getAttribute("href"), `/searchfox/rev/${REV}/tools/src/blame.rs#12`,
     "with a link to the code at the build's revision");
  is(find.querySelector(".crash-frame-name").tagName, "A", "and its function, found in the file");
  is(find.querySelector(".crash-frame-address")?.textContent, "searchfox-tool+0x12340", "Its module and offset");

  ok(impl.classList.contains("crash-frame-inlined") && crashText(impl).includes("find_phab_rev(iter.clone())"),
     "A call inlined in a frame has its own line");
  ok(crashText(patch).includes("commit_header_impl(commit, CommitHeaderResultKind::HeaderPatch)"),
     "and the frame its line");

  ok(where(info).includes("which has changed since"), "A line which changed since the build says so");
  is(info.querySelectorAll(".crash-line").length, 0, "and isn't highlighted");

  ok(crashText(deserialize).includes("line.splitn(4, ':')"), "A line which moved since is mapped");
  ok(/^At line 147 at the build's revision, which is line \d+ now\.$/.test(where(deserialize)),
     "and says where it is now");

  ok(libc.querySelector(".crash-code-missing").textContent.includes("No source"), "A frame without a file");
  is(libc.querySelector(".crash-frame-name").textContent, "libc.so.6+0x29d90", "is its module and offset");

  // Every thread's stack.
  const all = doc().querySelector("a.crash-all-threads");
  ok(all?.textContent.includes("all 2 threads' stacks"), "The view links to every thread's stacks");
  const url = new URL(all.href);
  await TestUtils.loadPath(url.pathname + url.search);
  is(doc().querySelector(".crash-thread-heading")?.textContent, "Thread #1 (Worker)", "which has the other thread's");
  const worker = doc().querySelector("#thread-1-frame-0");
  ok(crashText(worker).includes('format!("{}:{}:{}:", self.rev, self.path, self.lineno)'),
     "with its frames' lines");
  ok(doc().querySelector("a.crash-crashing-thread"), "and links back to the crashing thread's only");
});

add_task(async function test_CrashReportDwim() {
  // (A search for a crash-stats report's URL, or its ID as Bugzilla has
  // them, goes to its crash view's query at once, in a tree searching with
  // queries; "all" before the URL is every thread's.)
  for (const [input, query] of [
    [`https://crash-stats.mozilla.org/report/index/${ID}#tab-details`, `crash-report:${ID}`],
    [`bp-${ID}`, `crash-report:${ID}`],
    [`all https://crash-stats.mozilla.org/report/index/${ID}`, `crash-report:${ID} crash-threads:all`],
  ]) {
    await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(input)}`);
    is(doc().location.pathname, "/trydelta/query/default", `${input} goes to`);
    is(new URLSearchParams(doc().location.search).get("q"), query, "the query of its report's stack");
    ok(doc().querySelector(".crash-report-link")?.href.endsWith(`/report/index/${ID}`), "which links to it on crash-stats");
  }
});
