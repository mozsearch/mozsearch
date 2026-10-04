"use strict";

// Diffs have blame strips for both kinds of blame: the "searchfox" tree's are
// the token-centric blame's, and the "searchfox-line-blame" tree's the classic
// line blame's.
for (const tree of ["searchfox-line-blame", "searchfox"]) {
  add_task(async function test_BlameStripsInDiff() {
    await TestUtils.loadPath(`/${tree}/diff/b17c096ff1eab51aaf27befb5bd97ead09c74110/.gitignore`);

    {
      const blameStrip = frame.contentDocument.querySelector(`#line-1 .blame-strip`);
      ok(blameStrip.getBoundingClientRect().height > 0,
         `${tree}: Blame strip is visible for existing line`);
    }

    {
      const blameStrip = frame.contentDocument.querySelector(`#line-7 .blame-strip`);
      ok(blameStrip.getBoundingClientRect().height > 0,
         `${tree}: Blame strip is visible for newly added line`);
    }
  });
}

// A diff's token-centric blame has the blame of the file in the commit, for its
// lines, and in the commit's parent, for the lines it removed.  b472fa97
// (Bug 2013246) changed line 480 of head.js by adding a `return` to it.
add_task(async function test_TokenBlameInDiff() {
  await TestUtils.loadPath("/searchfox/diff/b472fa9717b98ce015ed72c5ff279637909068c9/tests/webtest/head.js");

  const doc = frame.contentDocument;
  const blameInfos = frame.contentWindow.BLAME_INFOS;
  is(blameInfos.length, 2, "The diff has the blame of the file in the commit and in its parent");
  ok(blameInfos[0].dataUrl.includes("/b472fa9717b98ce015ed72c5ff279637909068c9/"),
     "The first blame is of the file in the commit");

  const removed = doc.querySelector("code.minus-line").closest(".source-line-with-number")
    .querySelector(".blame-strip");
  is(removed.dataset.hbCtx, "1", "The removed line's strip is for the parent's blame");
  is(removed.dataset.hbLine, "480", "The removed line's strip is for the parent's line");
  const added = doc.querySelector("#line-480 .blame-strip");
  is(added.dataset.hbCtx, "0", "The changed line's strip is for the commit's blame");
  is(added.dataset.hbLine, "480", "The changed line's strip is for its line");

  const popup = doc.querySelector("#blame-popup");
  async function showPopup(strip) {
    frame.contentWindow.BlamePopup.triggerElement = null;
    TestUtils.dispatchMouseEvent("mouseenter", strip);
    await waitForCondition(
      () => popup.style.display != "none" &&
        frame.contentWindow.BlamePopup.popupOwner == strip &&
        popup.querySelector(".hb-popup"),
      "The token blame popup is shown");
    return {
      details: popup.querySelector(".hb-details").textContent,
      tokens: [...popup.querySelectorAll(".hb-replica .hb-token")].map(t => t.textContent),
    };
  }

  const before = await showPopup(removed);
  ok(!before.details.includes("Bug 2013246"), "The removed line's tokens are from before the commit");
  is(before.tokens[0], "this", "The removed line's tokens follow its origin");

  const after = await showPopup(added);
  ok(after.details.includes("Bug 2013246"), "The changed line has the commit's token");
  is(after.tokens[0], "return", "The changed line's tokens follow its origin");
});
