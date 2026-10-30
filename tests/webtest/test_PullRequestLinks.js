"use strict";

// A forest's commits' links to their GitHub pull requests and the issues
// those close, first in the blame popup's entries (see
// tools/src/pull_requests.rs): the specpr tree's spec repository's second
// trunk commit (which changed line 11 of its source, "clockwise") is pull
// request 9 of example/gizmo, closing its issue 3 and other/widgets' issue 4,
// by its merged pull requests' snapshot (see tests/specpr/setup), and its
// first commit has neither.
const PATH = "/specpr/source/gizmo/source";

add_task(async function test_PullRequestLinks() {
  await TestUtils.loadPath(PATH);
  const strip = frame.contentDocument.querySelector("#line-11 .blame-strip");
  const popup = frame.contentDocument.querySelector("#blame-popup");
  frame.contentWindow.BlamePopup.triggerElement = null;
  TestUtils.dispatchMouseEvent("mouseenter", strip);
  await waitForCondition(
    () => popup.style.display != "none" && popup.querySelectorAll(".hb-entry").length >= 2,
    "The popup is shown for line 11, with its commits' entries");

  const links = entry => [...entry.querySelectorAll(":scope > div > a")];
  const [newest, oldest] = popup.querySelectorAll(".hb-entry");
  ok(newest.textContent.includes("Say which way to twist gizmos"), "The newest commit is the trunk's second");
  is(links(newest).slice(0, 4).map(a => a.textContent).join(", "),
     "PR #9, issue #3, issue other/widgets#4, diff",
     "Its pull request and the issues it closes are first, then its diff");
  is(links(newest).slice(0, 3).map(a => a.getAttribute("href")).join(" "),
     "https://github.com/example/gizmo/pull/9 https://github.com/example/gizmo/issues/3 https://github.com/other/widgets/issues/4",
     "on GitHub");
  ok(oldest.textContent.includes("Add the Gizmo spec"), "The other commit is the trunk's first");
  is(links(oldest)[0]?.textContent, "diff", "which has neither");
});
