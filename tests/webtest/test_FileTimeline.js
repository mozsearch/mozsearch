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
