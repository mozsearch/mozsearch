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

  // Line 1's 9 tokens were last changed by 3 commits, whose rows are above the
  // replica of the line (oldest first) and whose details are below it (newest
  // first).
  {
    const popup = await showPopup(1);
    is(popup.querySelectorAll(".hb-row").length, 3, "Each commit has a row");
    const tokens = new Set([...popup.querySelectorAll(".hb-replica .hb-token")].map(t => t.dataset.token));
    is(tokens.size, 9, "Each token is in the replica");
    is(popup.querySelectorAll(".hb-lane").length, 9, "Each token has a lane");
    const entries = popup.querySelectorAll(".hb-entry");
    is(entries.length, 3, "Each commit has details");
    const links = [...popup.querySelectorAll(".hb-entry a")].filter(
      a => a.textContent == "earliest version with these tokens");
    is(links.length, 3, "Each commit links to its tokens");
    ok(links[0].getAttribute("href").startsWith("/searchfox/rev/b6d5e2737a4ad27651c30fa47d92a14248c1a95c/tools/src/blame.rs#tokens="),
       "The newest commit comes first");
    const rows = popup.querySelectorAll(".hb-row");
    is(rows[rows.length - 1].dataset.commit, entries[0].dataset.commit,
       "The newest commit's row is nearest the line");

    // Hovering over a token highlights its commit's row and details.
    const token = popup.querySelector(".hb-replica .hb-token");
    token.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    const commit = token.dataset.commit;
    ok(popup.querySelector(`.hb-row[data-commit="${commit}"]`).classList.contains("hb-hot"),
       "The token's commit's row is highlighted");
    ok(popup.querySelector(`.hb-entry[data-commit="${commit}"]`).classList.contains("hb-hot"),
       "The token's commit's details are highlighted");

    // Clicking on a token shows the context menu with links for it, rather
    // than the menu for searching for text.
    TestUtils.click(token);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "The context menu is shown");
    ok(menu.textContent.includes("Show the earliest version with this token"),
       "The context menu has the token's links");
    ok(!menu.textContent.includes("Search for"),
       "The context menu doesn't offer to search for the token's text");
    // The blame strip ignores hovers while a context menu is shown.
    frame.contentWindow.ContextMenu.hide();
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
  const link = [...popup.querySelectorAll(".hb-entry a")].find(
    a => a.textContent == "earliest version with these tokens");
  TestUtils.click(link);

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("/searchfox/rev/b6d5e2737a4ad27651c30fa47d92a14248c1a95c/") &&
      frame.contentWindow.location.hash == "#1",
    "Navigates to the commit with the line of its tokens selected");
});

add_task(async function test_BlameLens() {
  await TestUtils.loadPath(PATH);

  const doc = frame.contentDocument;
  const headings = [...doc.querySelectorAll("#panel h4")].map(h => h.textContent);
  ok(headings.indexOf("Lenses") == headings.indexOf("Copy as Markdown") + 1,
     "The Lenses section comes after Copy as Markdown");

  const select = doc.querySelector("#panel-lens-blame");
  const original = select.value;
  TestUtils.selectMenu(select, "author");
  ok(doc.documentElement.classList.contains("blame-colorized"),
     "Changing the lens colors the strip");
  is(JSON.parse(frame.contentWindow.localStorage.getItem("settings")).settings.blame.colorMode,
     "author", "Changing the lens changes the setting");

  // Pages loaded later use the setting.
  await TestUtils.loadPath(PATH);
  is(frame.contentDocument.querySelector("#panel-lens-blame").value, "author",
     "The lens shows the setting");
  ok(frame.contentDocument.documentElement.classList.contains("blame-colorized"),
     "The strip is colored by the setting");

  TestUtils.selectMenu(frame.contentDocument.querySelector("#panel-lens-blame"), original);
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
