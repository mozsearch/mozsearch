"use strict";

// A crash report's threads in its crash view (see tools/src/crash_threads.rs
// and config_defaults/crash-threads.toml, and crash_stack.rs's
// `crumbs_html`, `thread_facets`, `collapsed_html`): what each is and was
// doing (its kind, owner and state), its infrastructure's frames collapsed;
// the view's breadcrumbs (the report, and the thread, with a picker of the
// threads); and with every thread's, the navigation panel's facets of them.
// The report is tests/crash-reports/'s fixture, whose crashing thread's top
// frames are in the searchfox tree's tools/src/blame.rs, above its event
// loop, with threads idle at their waits, a DOM Worker shutting down, a
// TaskController pool's waiting in its own code, an active Gecko thread, and
// one without symbols.
const ID = "5e4c0b1a-7e57-4c0d-8e5a-000000261101";

function doc() {
  return frame.contentDocument;
}

const heading = index => {
  const section = doc().getElementById(`thread-${index}`);
  return {
    name: section?.querySelector(".crash-thread-name")?.textContent,
    kind: section?.querySelector(".crash-thread-kind")?.textContent,
    owner: section?.querySelector(".crash-thread-owner")?.textContent,
    state: section?.querySelector(".crash-thread-state")?.textContent,
  };
};
const shown = element => element && !element.closest("[hidden]");

add_task(async function test_CrashThreadsCrashing() {
  await TestUtils.loadQuery("searchfox", `crash-report:${ID}`);

  // The breadcrumbs: the report, and the crashing thread, with a link to
  // every thread's.
  const crumbs = doc().querySelector(".breadcrumbs .crash-crumbs");
  ok(crumbs && !crumbs.hidden, "The view's breadcrumbs are the page's");
  const report = crumbs.querySelector(".crash-crumb-report");
  is(report?.textContent, ID, "the report's ID first");
  is(report?.getAttribute("href"), `https://crash-stats.mozilla.org/report/index/${ID}`, "linked to crash-stats");
  is(crumbs.querySelector(".crash-crumb-thread")?.textContent, "Thread: #0 (MainThread)", "then the thread");
  const all = crumbs.querySelector(".crash-crumb-all");
  is(all?.textContent, "(with all threads)", "with a link to every thread's");
  ok(new URL(all.href).searchParams.get("q").endsWith("crash-threads:all"), "which adds crash-threads:all");
  ok(!doc().getElementById("crash-thread-picker"), "(but no picker, without them)");
  ok(!doc().querySelector(".facet-box"), "and no facets");

  // The report's menu.
  TestUtils.click(doc().getElementById("crash-report-menu-button"));
  const menu = doc().getElementById("crash-report-menu");
  is(menu.style.display, "flex", "The report's menu opens");
  is([...menu.querySelectorAll("a")].map(a => a.textContent).join("|"),
     "Open the report on crash-stats|Copy the report's ID|Its signature's stack|All 8 threads' stacks",
     "with crash-stats, its ID, its signature's stack and every thread's");
  TestUtils.dispatchMouseEvent("mousedown", doc().querySelector(".crash-notice"));
  is(menu.style.display, "none", "and closes");

  // The crashing thread: its top frames with their code, and its event loop
  // and its start collapsed, without code.
  is(JSON.stringify(heading(0)),
     JSON.stringify({ name: "Thread #0 (MainThread)", kind: "main thread", owner: "Core: XPCOM", state: "crashed" }),
     "The crashing thread is the main thread, crashed");
  const stack = doc().querySelector("#thread-0 .crash-stack");
  ok(doc().getElementById("frame-0")?.matches(".crash-frame") &&
     doc().getElementById("frame-0").querySelector(".crash-line")?.textContent.includes("line.strip_prefix(PREFIX)"),
     "Its own frames have their code");
  const collapsed = stack.querySelectorAll(".crash-collapsed");
  is(collapsed.length, 1, "Its event loop and its start are one collapsed line");
  is(collapsed[0].querySelector(".crash-collapsed-names")?.textContent, "XPCOM event loop › main thread start",
     "saying which layers");
  is(collapsed[0].querySelector(".crash-collapsed-count")?.textContent, "7 frames, #2–#8", "and which frames");
  ok(!collapsed[0].querySelector("details").open, "closed at first");
  const loop = doc().getElementById("frame-2");
  ok(loop?.matches("li.crash-compact-frame") && loop.textContent.includes("nsThread::ProcessNextEvent(bool, bool*)"),
     "Its frames are a line each");
  ok(loop.textContent.includes("xpcom/threads/nsThread.cpp:1199") && loop.textContent.includes("XPCOM event loop"),
     "saying where, and their layers");
  is(collapsed[0].querySelectorAll(".crash-rows").length, 0, "without their code");
});

add_task(async function test_CrashThreadsAll() {
  await TestUtils.loadQuery("searchfox", `crash-report:${ID} crash-threads:all`);
  // What each thread is and was doing.
  for (const [index, expected] of [
    [1, { name: "Thread #1 (Timer)", kind: "timer thread", owner: "Core: XPCOM", state: "idle" }],
    [2, { name: "Thread #2 (DOM Worker)", kind: "DOM Worker", owner: "Core: DOM: Workers", state: "shutting down" }],
    [3, { name: "Thread #3 (TaskController #1)", kind: "TaskController pool", owner: "Core: XPCOM",
          state: "waiting (condition variable)" }],
    [4, { name: "Thread #4 (StyleThread#0)", kind: "Stylo worker", owner: "Core: CSS Parsing and Computation",
          state: "idle" }],
    [5, { name: "Thread #5 (Link Monitor)", kind: "Gecko thread", owner: "Gecko", state: "active" }],
    [6, { name: "Thread #6 (TaskController #2)", kind: "TaskController pool", owner: "Core: XPCOM", state: "idle" }],
    [7, { name: "Thread #7", kind: "system thread", owner: "System", state: "unknown" }],
  ]) {
    is(JSON.stringify(heading(index)), JSON.stringify(expected), `Thread #${index} is ${expected.kind}, ${expected.state}`);
  }

  // An idle thread is one line, its frames its wait and its infrastructure.
  const timer = doc().querySelectorAll("#thread-1 .crash-stack > *");
  is(timer.length, 1, "An idle thread's frames are one line");
  is(timer[0].querySelector(".crash-collapsed-names")?.textContent,
     "its wait › its idle point › XPCOM event loop › NSPR › thread start", "its wait, idle point and layers");
  // A waiting thread: its wait collapsed, then its own frames, then its
  // infrastructure.
  const waiting = [...doc().querySelectorAll("#thread-3 .crash-stack > *")];
  is(waiting.map(row => row.className).join(" "), "crash-collapsed crash-frame crash-collapsed",
     "A waiting thread's wait and its infrastructure are collapsed, around its own frame");
  is(waiting[0].querySelector(".crash-collapsed-names")?.textContent, "waiting (condition variable)", "its wait");
  ok(waiting[1].querySelector(".crash-line")?.textContent.includes('format!("{}:{}:{}:", self.rev, self.path, self.lineno)'),
     "its own frame with its code");
  // Frames without code, of threads which didn't crash, are a line each.
  const worker = doc().querySelector("#thread-2 .crash-codeless");
  ok(worker?.textContent.includes("NS_ProcessPendingEvents(nsIThread*, mozilla::TimeDuration)"),
     "Other threads' frames without code are a line each");

  // The picker of the threads, as the tree switcher's, their kinds its
  // headings.
  const crumb = doc().querySelector(".crash-crumb-thread");
  const picker = doc().getElementById("crash-thread-picker");
  ok(picker, "With every thread's, the thread's breadcrumb has a picker");
  TestUtils.click(picker);
  const menu = doc().getElementById("crash-thread-menu");
  is(menu.style.display, "flex", "which opens");
  const labels = [...menu.querySelectorAll(".context-menu-group-label")].map(label => label.firstChild.textContent);
  is(labels[0], "main thread", "the crashed thread's kind first");
  ok(labels.includes("TaskController pool") && labels.includes("Stylo worker"), "headed by the threads' kinds");
  is(menu.querySelectorAll("a[data-thread]").length, 8, "with every thread");
  is(menu.querySelector('a[data-thread="3"]')?.textContent, "#3 (TaskController #1) waiting (condition variable)",
     "and its state");
  TestUtils.dispatchMouseEvent("mousedown", doc().querySelector(".crash-notice"));

  // The navigation panel's facets of the threads (but the crashed one,
  // which always shows): their states (waits within waiting), kinds and
  // owners (nested).
  const value = (facet, v) =>
    doc().querySelector(`.facet-box .facet[data-facet="${facet}"] .facet-value[data-value="${v}"]`);
  const count = (facet, v) => value(facet, v)?.querySelector(".facet-count")?.textContent;
  is(doc().querySelector(".facet-box")?.dataset.nouns, "threads", "The panel's facets are the threads'");
  is(count("state", "idle"), "3", "counting the idle threads");
  is(count("state", "waiting"), "1", "the waiting ones");
  is(count("state", "waiting-condition-variable"), "1", "by their waits");
  is(count("kind", "taskcontroller-pool"), "2", "the threads of each kind");
  is(count("owner", "core"), "5", "and of each owner");
  is(count("owner", "core-xpcom"), "3", "nested by its components");

  // Leaving out the idle threads, and a kind's.
  TestUtils.click(value("state", "idle"), { bubbles: true, shiftKey: true });
  ok(!shown(doc().getElementById("thread-1")) && !shown(doc().getElementById("thread-6")),
     "Leaving out idle hides the idle threads");
  ok(shown(doc().getElementById("thread-3")) && shown(doc().getElementById("thread-0")), "but not the others");
  const params = () => new URLSearchParams(doc().location.search);
  is(JSON.stringify(params().getAll("-facet-state")), JSON.stringify(["idle"]), "and the URL says so");
  TestUtils.click(value("kind", "taskcontroller-pool"), { bubbles: true, shiftKey: true });
  ok(!shown(doc().getElementById("thread-3")), "A kind's facet hides its threads");
  is(JSON.stringify(params().getAll("-facet-kind")), JSON.stringify(["taskcontroller-pool"]), "also in the URL");

  // Picking a hidden thread shows it.
  TestUtils.click(picker);
  TestUtils.click(menu.querySelector('a[data-thread="1"]'));
  ok(shown(doc().getElementById("thread-1")), "Picking a hidden thread shows it");
  is(doc().location.hash, "#thread-1", "and goes to it");
  is(crumb.textContent, "Thread: #1 (Timer)", "which the breadcrumb says");

  // The URL's facets hide threads at first.
  await TestUtils.loadQuery("searchfox", `crash-report:${ID} crash-threads:all`, { "-facet-state": "idle" });
  ok(value("state", "idle").classList.contains("facet-excluded"), "A URL's left out facets are left out");
  ok(doc().getElementById("thread-1").hidden && !doc().getElementById("thread-3").hidden,
     "and their threads hidden at first");
  ok(!doc().getElementById("thread-0").hidden, "(but never the crashing one)");
});
