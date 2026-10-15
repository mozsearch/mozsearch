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
