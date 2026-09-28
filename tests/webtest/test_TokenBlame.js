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

add_task(async function test_TokenHistory() {
  await TestUtils.loadPath(PATH);

  // Line 12's `strip_prefix` replaced `starts_with` in a clippy fix, and the
  // condition was introduced by the commit before that.
  const popup = await showPopup(12);
  const token = [...popup.querySelectorAll(".hb-replica .hb-token")].find(
    t => t.textContent == "strip_prefix");
  TestUtils.click(token);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "The context menu is shown");
  const item = [...menu.querySelectorAll("a")].find(
    a => a.textContent == "Follow this token into the past");
  TestUtils.click(item);

  await waitForCondition(
    () => popup.querySelector(".hb-history-status")?.textContent.includes("introduced all of the tokens followed"),
    "The history ends with the commit which introduced the condition");
  const steps = popup.querySelectorAll(".hb-step");
  is(steps.length, 2, "The history has 2 steps");
  ok(steps[1].textContent.includes("cargo clippy --fix"),
     "The newest step is nearest the line");
  ok(steps[0].querySelector(".hb-step-code").textContent.includes("if line.starts_with(PREFIX) {"),
     "The oldest step has the original condition");
  const changed = [...steps[1].querySelectorAll(".hb-step-changed")].map(s => s.textContent);
  ok(changed.includes("strip_prefix"), "The newest step highlights the tokens it introduced");

  frame.contentWindow.BlameStripHoverHandler.keepVisible = false;
  frame.contentWindow.BlamePopup.triggerElement = null;
});

async function followFromTokenMenu(lineno, tokenText, itemText) {
  const popup = await showPopup(lineno);
  const token = [...popup.querySelectorAll(".hb-replica .hb-token")].find(
    t => t.textContent == tokenText);
  TestUtils.click(token);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "The context menu is shown");
  const item = [...menu.querySelectorAll("a")].find(a => a.textContent == itemText);
  TestUtils.click(item);
  return popup;
}

add_task(async function test_LineHistory() {
  await TestUtils.loadPath(PATH);

  // Following line 12 follows its identifiers, including `strip_prefix`
  // back to the `starts_with` it replaced.
  const popup = await followFromTokenMenu(12, "strip_prefix", "Follow this line into the past");
  await waitForCondition(
    () => popup.querySelector(".hb-history-status")?.textContent.includes("introduced all of the tokens followed"),
    "The history ends with the commit which introduced the condition");
  const steps = popup.querySelectorAll(".hb-step");
  is(steps.length, 2, "The history has 2 steps");
  const followed = [...steps[0].querySelectorAll(".hb-step-anchor")].map(s => s.textContent);
  is(followed.join(" "), "line starts_with PREFIX", "The oldest step follows the line's identifiers");

  frame.contentWindow.BlameStripHoverHandler.keepVisible = false;
  frame.contentWindow.BlamePopup.triggerElement = null;
});

add_task(async function test_TokenFuture() {
  // In the commit which introduced `find_phab_rev`, its condition used
  // `starts_with`, which a clippy fix later changed into `strip_prefix`.
  await TestUtils.loadPath("/searchfox/rev/681ef2a9293f916c9b9f20a58562cf77e3f18b12/tools/src/blame.rs");
  const popup = await followFromTokenMenu(16, "starts_with", "Follow this token into the future");
  await waitForCondition(
    () => popup.querySelector(".hb-history-status")?.textContent.includes("This token is now"),
    "The token's location now is shown");
  const link = popup.querySelector(".hb-history-status a");
  ok(link.getAttribute("href").startsWith("/searchfox/source/tools/src/blame.rs#tokens="),
     "The link goes to the token in the latest version");
  const changes = popup.querySelectorAll(".hb-step");
  is(changes.length, 1, "One commit changed the token");
  ok(changes[0].textContent.includes("cargo clippy --fix") &&
     changes[0].textContent.includes("changed it into another token"),
     "The clippy fix changed the token");

  frame.contentWindow.BlameStripHoverHandler.keepVisible = false;
  frame.contentWindow.BlamePopup.triggerElement = null;
});

add_task(async function test_LatestVersionWithoutToken() {
  await TestUtils.loadPath(PATH);

  // `strip_prefix` replaced `starts_with` in a clippy fix, so the latest
  // version without it is the fix's parent, at `starts_with`.
  await followFromTokenMenu(12, "strip_prefix", "Show the latest version without this token");
  await waitForCondition(
    () => frame.contentDocument.location.href.includes("/searchfox/rev/26279563bf84400343c7a717764887c0fba7564c/tools/src/blame.rs") &&
      frame.contentDocument.querySelector(".highlighted"),
    "Navigates to the parent of the commit which introduced the token");
  ok(frame.contentDocument.querySelector(".highlighted code").textContent.includes("line.starts_with(PREFIX)"),
     "The token it replaced is selected");
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
