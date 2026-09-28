"use strict";

// The /explore/ pages show the commits of bugs (or Phabricator revisions) and
// a dense "horizontal Southern blot" of what they changed: a sparkline with a
// slot per commit for each file and symbol.  Bug 1975601's 4 parts landed
// together, so they share a commit time and are ordered by ancestry.
add_task(async function test_ExploreBug() {
  await TestUtils.loadPath("/searchfox/explore/bug/1975601");

  const doc = frame.contentDocument;
  const commits = [...doc.querySelectorAll(".explore-commit-summary")].map(h => h.textContent);
  is(commits.length, 4, "The bug has 4 commits");
  for (let i = 0; i < 4; i++) {
    ok(commits[i].includes(`Part ${i + 1}:`), `Part ${i + 1} is commit ${i + 1}`);
  }

  const files = [...doc.querySelectorAll(".explore-file")];
  const blameRs = files.find(f => f.querySelector(".explore-file-header a").textContent == "tools/src/blame.rs");
  ok(blameRs, "blame.rs has a sparkline");
  // With 4 commits, each commit's slot is 6px wide, and Part 1 is the first.
  ok(blameRs.querySelector('.explore-file-header rect[class^="explore-l"][x="0"]'),
     "Part 1 changed blame.rs");
  const symbol = [...blameRs.querySelectorAll(".explore-symbol")].find(
    s => s.querySelector(".explore-name").textContent == "find_phab_rev");
  ok(symbol, "The function Part 1 added has a sparkline");

  // Hovering over a slot shows its commit.
  const svg = blameRs.querySelector(".explore-file-header svg");
  const rect = svg.getBoundingClientRect();
  svg.dispatchEvent(new MouseEvent("mousemove", {
    bubbles: true,
    clientX: rect.left + 1,
    clientY: rect.top + 1,
  }));
  ok(doc.querySelector(".explore-status").textContent.includes("Commit 1:"),
     "The status says which commit is under the mouse");
  ok(doc.querySelector("#commit-1").classList.contains("explore-current"),
     "The commit is highlighted in the list");
});

// Classes are collapsed into their sparkline with a thin row per member, with
// a disclosure triangle and a count of their members.
add_task(async function test_ExploreCollapsedClass() {
  await TestUtils.loadPath("/searchfox/explore/bug/2002694");

  const doc = frame.contentDocument;
  const parent = [...doc.querySelectorAll(".explore-parent")].find(
    p => p.querySelector(".explore-name").textContent == "IndexConsumer");
  ok(parent, "IndexConsumer is collapsible");
  ok(!parent.open, "IndexConsumer is collapsed");
  const summary = parent.querySelector("summary");
  is(window.getComputedStyle(summary).display, "list-item", "The summary has a disclosure triangle");
  const count = parent.querySelector(".explore-count");
  is(count.textContent, "(7)", "The summary says how many members there are");
  ok(parent.querySelector(".explore-collapsed svg").getBoundingClientRect().height > 0,
     "The collapsed sparkline is visible");
  ok(!parent.querySelector(".explore-children").checkVisibility(), "The members are hidden");

  summary.click();
  await waitForCondition(() => parent.open, "IndexConsumer expands");
  ok(!count.checkVisibility(), "The count is hidden once the members are shown");
  const members = [...parent.querySelectorAll(".explore-child .explore-name")].map(n => n.textContent);
  is(members.length, 7, "The members are shown");
  ok(members.includes("combineRanges"), "The members don't have the class prefix");
});

add_task(async function test_ExploreMissingBug() {
  await TestUtils.loadPath("/searchfox/explore/bug/9999999");
  ok(frame.contentDocument.querySelector(".explore-note").textContent.includes("No commits mention 9999999"),
     "The page says when a bug has no commits");
});
