"use strict";

// The token-centric blame of the "searchfox" tree, which has history.  (The
// "searchfox-line-blame" tree covers the classic line blame.)  We use a fixed
// revision of blame.rs because its lines have tokens from several commits,
// removals within and between lines, and a blank line colored by its
// neighbors.
const REV = "50df75fc5253890347bfac70037bc7ad42316862";
const PATH = `/searchfox/rev/${REV}/tools/src/blame.rs`;

function strip(lineno) {
  return frame.contentDocument.querySelector(`#line-${lineno} .blame-strip`);
}

async function showPopup(lineno) {
  const popup = frame.contentDocument.querySelector("#blame-popup");
  // Make sure we see the new content rather than the previous line's.
  frame.contentWindow.BlamePopup.triggerElement = null;
  TestUtils.dispatchMouseEvent("mouseenter", strip(lineno));
  await waitForCondition(
    () => popup.style.display != "none" &&
      frame.contentWindow.BlamePopup.popupOwner == strip(lineno),
    `The popup is shown for line ${lineno}`);
  return popup;
}

add_task(async function test_TokenBlameStrip() {
  await TestUtils.loadPath(PATH);

  ok(frame.contentWindow.BLAME_INFO, "The page has BLAME_INFO");
  ok(strip(1).dataset.hyperblame, "The strip has token blame data");
  ok(strip(6).classList.contains("scar-below"), "Line 6 has a removal after it");
  ok(strip(6).classList.contains("scar-within"), "Line 6 has a removal within it");
  ok(strip(7).classList.contains("blame-interpolated"), "Blank line 7 is colored by its neighbors");
  is(strip(7).dataset.hyperblame, "", "Blank line 7 has no tokens");
});

add_task(async function test_TokenBlamePopup() {
  await TestUtils.loadPath(PATH);

  // Line 1's tokens were last changed by 3 commits.
  {
    const popup = await showPopup(1);
    ok(popup.textContent.includes("last changed in 3 commits"),
       "The popup says how many commits changed the line");
    const links = [...popup.querySelectorAll("a")].filter(
      a => a.textContent == "Show earliest version with these tokens");
    is(links.length, 3, "Each commit links to its tokens");
    ok(links[0].getAttribute("href").startsWith("/searchfox/rev/b6d5e2737a4ad27651c30fa47d92a14248c1a95c/tools/src/blame.rs#tokens="),
       "The newest commit comes first");
  }

  // The removal after line 6 is described for line 6 and the line after it.
  {
    const popup = await showPopup(6);
    ok(popup.textContent.includes("135 tokens were removed between this line and the next line (135 moved elsewhere)"),
       "The removal after the line is described");
    ok(popup.textContent.includes("removed within this line"),
       "The removal within the line is described");
  }
  {
    const popup = await showPopup(7);
    ok(popup.textContent.includes("This line has no tokens"),
       "The blank line has no tokens");
    ok(popup.textContent.includes("colored like the lines around it"),
       "The blank line is colored by its neighbors");
    ok(popup.textContent.includes("135 tokens were removed between the previous line and this line"),
       "The removal before the line is described");
  }
});

add_task(async function test_TokenHash() {
  // Token 5 is on line 1 and token 12 is on line 2.
  await TestUtils.loadPath(`${PATH}#tokens=5,12`);
  await waitForCondition(() => frame.contentWindow.location.hash == "#1-2",
                         "The tokens hash becomes a line selection");
  ok(frame.contentDocument.querySelector("#line-1").classList.contains("highlighted"),
     "Line 1 is selected");
  ok(frame.contentDocument.querySelector("#line-2").classList.contains("highlighted"),
     "Line 2 is selected");
});

add_task(async function test_TokenBlameEarliestVersionLink() {
  await TestUtils.loadPath(PATH);

  const popup = await showPopup(1);
  const link = [...popup.querySelectorAll("a")].find(
    a => a.textContent == "Show earliest version with these tokens");
  TestUtils.click(link);

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("/searchfox/rev/b6d5e2737a4ad27651c30fa47d92a14248c1a95c/") &&
      frame.contentWindow.location.hash == "#1",
    "Navigates to the commit with the line of its tokens selected");
});

add_task(async function test_TokenBlameColors() {
  await TestUtils.loadPath(PATH);

  const colorizer = frame.contentWindow.BlameColorizer;
  const alternating = frame.contentWindow.getComputedStyle(strip(1)).backgroundColor;
  colorizer.apply("age");
  ok(colorizer.style.textContent.includes(".bc-0 {"), "There are rules for the commits");
  isnot(frame.contentWindow.getComputedStyle(strip(1)).backgroundColor, alternating,
        "The strip is colored by age");
  colorizer.apply("alternating");
  is(frame.contentWindow.getComputedStyle(strip(1)).backgroundColor, alternating,
     "The strip alternates colors again");
});
