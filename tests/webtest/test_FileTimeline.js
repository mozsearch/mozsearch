"use strict";

// The file timeline replaces the revision's description under the search box
// (see file-timeline.js).
add_task(async function test_FileTimeline() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const doc = frame.contentDocument;
  const timeline = doc.querySelector("#revision button.file-timeline");
  ok(timeline, "The revision line has the file's timeline");
  ok(!doc.querySelector("#rev-desc"), "instead of the revision's description");
  const commits = frame.contentWindow.FileTimeline.commits.length;
  ok(commits > 1, "The file has commits");
  ok(timeline.querySelector(".file-timeline-label").textContent.startsWith(`${commits} commits, `),
     "The timeline says how many");
  is(timeline.querySelectorAll(".file-timeline-tick").length, commits, "with a tick for each");

  TestUtils.click(timeline);
  const popup = doc.querySelector(".file-timeline-popup");
  ok(popup, "Clicking it shows the file's history");
  await waitForCondition(() => popup.querySelectorAll(".file-timeline-commits li").length == commits,
                         "with each commit");
  const item = popup.querySelector(".file-timeline-commits li");
  ok(item.querySelector("a").getAttribute("href").startsWith("/searchfox/commit/"),
     "which links to the commit");
  item.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  ok(popup.querySelector(`.file-timeline-tick[data-commit="${item.dataset.commit}"]`)
       .classList.contains("file-timeline-highlight"),
     "Hovering a commit highlights its tick");
  TestUtils.click(popup.querySelector(".lens-popup-close"));
  ok(!doc.querySelector(".file-timeline-popup"), "Close closes it");
});

// The file's history's box searches it: a `text-history:` query of the file
// (see test_HistoryGrep.js for `structopt`, which tools/Cargo.toml lost).
add_task(async function test_FileTimelineSearch() {
  await TestUtils.loadPath("/searchfox/source/tools/Cargo.toml");
  const doc = frame.contentDocument;
  TestUtils.click(doc.querySelector("#revision button.file-timeline"));
  const input = doc.querySelector(".file-timeline-popup .file-timeline-header input");
  is(input?.placeholder, "Search this file's history",
     "The file's history has a box beside its heading searching it");
  input.value = "structopt";
  const loaded = TestUtils.waitForLoad();
  input.form.requestSubmit();
  await loaded;
  const page = frame.contentDocument;
  is(page.location.pathname, "/searchfox/query/default", "Submitting it queries");
  is(new URLSearchParams(page.location.search).get("q"),
     "text-history:structopt path:^tools/Cargo.toml$",
     "the file's history for the text");
  await waitForCondition(() => page.querySelector(".history-grep"), "The view is there");
  const rows = [...page.querySelectorAll(".history-grep-table tbody tr")];
  ok(rows.length > 0, "with the file's revisions changing its matches");
  ok(rows.every(row => row.textContent.includes("tools/Cargo.toml")), "and only the file's");
});

// A permalink to a revision at which the file is the same as now goes to its
// source listing, which says so, with a link to exactly that revision.
add_task(async function test_PermalinkToSameFile() {
  // (The last commit changing LICENSE.)
  const rev = "c7eb2c58405b87e15c3951d0bac05fe1b04a2fe2";
  await TestUtils.loadPath(`/searchfox/rev/${rev}/LICENSE#3`);
  const loc = frame.contentDocument.location;
  is(loc.pathname, "/searchfox/source/LICENSE", "The permalink goes to the source listing");
  is(loc.search, "", "whose URL is its own");
  is(loc.hash, "#3", "with the permalink's lines");
  const note = frame.contentDocument.querySelector(".permalink-note");
  ok(note?.textContent.includes(rev.slice(0, 12)), "It says it's the same file as at the permalink's revision");
  const exact = note.querySelector("a");
  is(exact.getAttribute("href"), `/searchfox/rev/${rev}/LICENSE?exactly#3`,
     "with a link to exactly that revision");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(exact);
  await loadPromise;
  is(frame.contentDocument.location.pathname, `/searchfox/rev/${rev}/LICENSE`,
     "which shows the file at that revision");
  ok(!frame.contentDocument.querySelector(".permalink-note"), "without the note");
});

// A permalink to a revision at which the file has changed since offers to
// follow its selection into the future.
add_task(async function test_PermalinkFuture() {
  // (The parent of the file's latest change, at which it differs from now.)
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const timeline = frame.contentWindow.FileTimeline;
  const newest = timeline.commits[timeline.commits.length - 1];
  const headers = await (await frame.contentWindow.fetch(`${timeline.info.dataUrl}/commits.json`)).json();
  const parent = headers[newest.index].parent;
  await TestUtils.loadPath(`/searchfox/rev/${parent}/tools/src/format.rs#1`);
  const note = frame.contentDocument.querySelector(".permalink-note");
  ok(note?.textContent.includes("which has changed since"), "The page says the file has changed since");
  TestUtils.click(note.querySelector("button"));
  await waitForCondition(
    () => /It's now here|It was removed by|couldn't follow/.test(note.querySelector(".permalink-future").textContent),
    "Following the line's first token says where it went");
});

// A permalink copied with a token selected has the token, which a later page
// of the file at the permalink's revision follows into the future.
add_task(async function test_TokenPermalink() {
  await TestUtils.loadPath("/searchfox/source/tools/src/output.rs");
  const doc = frame.contentDocument;
  const clipboard = TestUtils.spyClipboard();
  const token = doc.querySelector('code.source-line span[data-symbols]');
  TestUtils.click(token);
  await waitForCondition(() => frame.contentWindow.Panel.selectedTokenIndex > 0,
                         "Selecting a token finds its index");
  const index = frame.contentWindow.Panel.selectedTokenIndex;
  frame.contentWindow.ContextMenu.hide();
  TestUtils.click(doc.querySelector("#panel-copy-symbol-link"));
  ok(clipboard.value.includes(`?token=${index}#`),
     "The copied permalink has the token, before the line hash");

  // (The parent of the file's latest change, at which it differs from now.)
  const timeline = frame.contentWindow.FileTimeline;
  const newest = timeline.commits[timeline.commits.length - 1];
  const headers = await (await frame.contentWindow.fetch(`${timeline.info.dataUrl}/commits.json`)).json();
  await TestUtils.loadPath(`/searchfox/rev/${headers[newest.index].parent}/tools/src/output.rs?token=1#1`);
  const note = frame.contentDocument.querySelector(".permalink-note");
  TestUtils.click(note.querySelector("button"));
  await waitForCondition(
    () => /It's now here|It was removed by|couldn't follow/.test(note.querySelector(".permalink-future").textContent),
    "The permalink's token is followed");
});

// The timeline's commits link to their diffs of the file and the file then,
// and can highlight the changes since them on the blame strip.
add_task(async function test_TimelineSince() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const doc = frame.contentDocument;
  TestUtils.click(doc.querySelector("#revision button.file-timeline"));
  const popup = doc.querySelector(".file-timeline-popup");
  await waitForCondition(() => popup.querySelector(".file-timeline-commits li .file-timeline-extras"),
                         "The commits are listed");
  const item = popup.querySelector(".file-timeline-commits li");
  const links = [...item.querySelectorAll(".file-timeline-extras a")].map(a => a.getAttribute("href"));
  ok(links[0].startsWith("/searchfox/diff/") && links[0].endsWith("/tools/src/format.rs"),
     "A commit links to its diff of the file");
  ok(links[1].startsWith("/searchfox/rev/") && links[1].endsWith("/tools/src/format.rs"),
     "and to the file then");
  TestUtils.click(item.querySelector(".file-timeline-extras button"));
  ok(popup.querySelector(".file-timeline-since").textContent.startsWith("Highlighting the changes since"),
     "Since highlights the changes since it");
  ok(frame.contentWindow.BlameColorizer.sinceTime > 0, "on the strip");
  TestUtils.click(popup.querySelector(".file-timeline-since button"));
  ok(!frame.contentWindow.BlameColorizer.sinceTime, "Clear clears it");
  TestUtils.click(popup.querySelector(".lens-popup-close"));
});

// The timeline's ticks are as tall as their commits' changes to the file are
// big, and its list says what each commit changed most.
add_task(async function test_TimelineChanges() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const doc = frame.contentDocument;
  const timeline = frame.contentWindow.FileTimeline;
  const changes = timeline.info.changes;
  is(changes?.length, timeline.info.commits.length, "The page has each commit's change to the file");
  ok(changes.some(change => change > 0), "with their sizes");
  const button = doc.querySelector("#revision button.file-timeline");
  const heights = new Set([...button.querySelectorAll(".file-timeline-tick")].map(tick => tick.getAttribute("y1")));
  ok(heights.size > 1, "The ticks' heights vary");
  ok(timeline.commits.every(commit => timeline.tickHeight(commit) >= 0.2 && timeline.tickHeight(commit) <= 0.8),
     "between 20% and 80% of the timeline's");

  TestUtils.click(button);
  const popup = doc.querySelector(".file-timeline-popup");
  await waitForCondition(() => popup.querySelector(".file-timeline-change"),
                         "The list says what the commits changed");
  ok(popup.classList.contains("file-timeline-with-changes"), "in a column of their own");
  ok(Math.abs(popup.getBoundingClientRect().left - doc.querySelector("#rev-id").getBoundingClientRect().left) < 1,
     "The popup starts at the page's indent");
  const change = popup.querySelector(".file-timeline-change");
  ok(/tokens added/.test(change.title), "A change says how many tokens it added and removed");
  TestUtils.click(popup.querySelector(".lens-popup-close"));
});

// Firefox's version cuts are in a band above the commits' ticks, labeled as
// they fit; pointing in the band says the nearest one's date, and clicking
// highlights the changes since it.  (The tests' trees have no
// config/milestone.txt, so these are made up.)
add_task(async function test_TimelineCuts() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const doc = frame.contentDocument;
  const timeline = frame.contentWindow.FileTimeline;
  const span = timeline.end - timeline.start;
  const cuts = [0.2, 0.4, 0.6].map((at, i) => ({
    version: 100 + i,
    rev: "a".repeat(40),
    parent: "b".repeat(40),
    time: Math.round(timeline.start + at * span),
  }));
  timeline.cutsPromise = Promise.resolve(cuts);
  TestUtils.click(doc.querySelector("#revision button.file-timeline"));
  const popup = doc.querySelector(".file-timeline-popup");
  await waitForCondition(() => popup.querySelectorAll(".file-timeline-release").length == 3,
                         "The cuts are in the timeline");
  is([...popup.querySelectorAll(".file-timeline-versions span")].map(label => label.textContent).join(","),
     "100,101,102", "labeled");
  const svg = popup.querySelector(".file-timeline-big svg");
  const box = svg.getBoundingClientRect();
  const at = { clientX: box.left + box.width * 0.41, clientY: box.top + 2, bubbles: true };
  popup.querySelector(".file-timeline-big").dispatchEvent(new frame.contentWindow.PointerEvent("pointermove", at));
  ok(popup.querySelector(".file-timeline-readout").textContent.startsWith("Firefox 101: cut from mozilla-central on"),
     "Pointing in the band says the nearest cut");
  ok(popup.querySelector('.file-timeline-release[data-cut="1"]').classList.contains("file-timeline-highlight"),
     "and highlights it");
  popup.querySelector(".file-timeline-big").dispatchEvent(new frame.contentWindow.MouseEvent("click", at));
  ok(popup.querySelector(".file-timeline-since").textContent.startsWith("Highlighting the changes since Firefox 101"),
     "Clicking highlights the changes since it");
  is(frame.contentWindow.BlameColorizer.sinceTime, cuts[1].time, "on the strip");
  ok(popup.querySelector(".file-timeline-since a").getAttribute("href").includes(`/outline-diff/${cuts[1].parent}/`),
     "and compares the outlines from the version's last commit");
  TestUtils.click(popup.querySelector(".file-timeline-since button"));
  TestUtils.click(popup.querySelector(".lens-popup-close"));
});
