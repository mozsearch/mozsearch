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
