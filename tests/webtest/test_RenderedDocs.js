"use strict";

// The rendered views of documents, whose blocks have blame strips, in the
// "searchfox" tree, which has history.  We use a fixed revision of README.md,
// whose blocks have tokens from several commits.
const REV = "09d16a31acbca05ddd0befd6bfadd9bff67b8f1a";
const PATH = `/searchfox/rev-rendered/${REV}/README.md`;

function doc() {
  return frame.contentDocument.querySelector("#rendered-doc");
}

// The strip of the first block whose text starts with `text`.
function stripOf(text) {
  for (const strip of doc().querySelectorAll(".blame-strip")) {
    const block = strip.closest("tr") || strip.parentElement;
    if (block.textContent.trim().startsWith(text)) {
      return strip;
    }
  }
  return null;
}

async function showPopup(strip, clientY) {
  const popup = frame.contentDocument.querySelector("#blame-popup");
  frame.contentWindow.BlamePopup.triggerElement = null;
  TestUtils.dispatchMouseEvent("mouseenter", strip, { bubbles: true, clientY });
  await waitForCondition(
    () => popup.style.display != "none" &&
      frame.contentWindow.BlamePopup.popupOwner == strip &&
      popup.querySelector(".rd-replica"),
    "The popup is shown for the block");
  return popup;
}

// The words of the replica's tokens.  (Prose's punctuation is tokens of its
// own.)
function replicaTokens(popup) {
  return [...popup.querySelectorAll(".rd-replica .hb-token")].map(t => t.textContent);
}

add_task(async function test_RenderedDocument() {
  await TestUtils.loadPath(PATH);

  ok(frame.contentWindow.BLAME_INFO, "The page has BLAME_INFO");
  is(doc().querySelector("h1").textContent, "Mozsearch", "The title is a heading");
  ok(doc().querySelector('a[href="https://searchfox.org"]'), "Links are links");
  ok(doc().querySelector(".rd-admonition.rd-warning"), "GitHub's alerts are admonitions");

  // Each block has a strip, in the gutter wherever the block is nested.
  const title = stripOf("Mozsearch");
  ok(title.dataset.hyperblame, "The title's strip has token blame data");
  is(title.dataset.rdLines, "1-1", "The title is line 1");
  const item = doc().querySelector("li .blame-strip");
  ok(item, "List items have strips");
  ok(Math.abs(item.getBoundingClientRect().left - title.getBoundingClientRect().left) < 1,
     "Nested blocks' strips line up with the others");

  // The panel links to the source listing, which links back.
  const source = frame.contentDocument.querySelector("#panel-source-listing");
  is(source.getAttribute("href"), `/searchfox/rev/${REV}/README.md`,
     "The panel links to the source listing");
  await TestUtils.loadPath(`/searchfox/rev/${REV}/README.md`);
  is(frame.contentDocument.querySelector("#panel-rendered-view").getAttribute("href"), PATH,
     "The source listing links to the rendered view");
});

add_task(async function test_RenderedBlockPopup() {
  await TestUtils.loadPath(PATH);

  // The first paragraph wraps, and its popup is for the visual line under
  // the mouse.
  const strip = stripOf("Mozsearch is the backend");
  ok(strip, "The paragraph has a strip");
  const rect = strip.getBoundingClientRect();
  {
    const popup = await showPopup(strip, rect.top + 2);
    const tokens = replicaTokens(popup);
    ok(tokens.includes("Mozsearch"), "The first visual line's tokens are in the replica");
    ok(!tokens.includes("Vagrant"), "The last visual line's tokens aren't");
    ok(popup.querySelectorAll(".hb-row").length >= 1, "The line's commits have rows");
    const shown = new Set([...popup.querySelectorAll(".rd-replica .hb-token")].map(t => t.dataset.token));
    is(popup.querySelectorAll(".hb-lane").length, shown.size, "Each token has a lane");
  }

  // Moving the mouse down the strip shows the last visual line.
  {
    const popup = frame.contentDocument.querySelector("#blame-popup");
    TestUtils.dispatchMouseEvent("mousemove", strip, { bubbles: true, clientY: rect.bottom - 2 });
    await waitForCondition(() => replicaTokens(popup).includes("Vagrant"),
                           "The popup shows the last visual line");
    ok(!replicaTokens(popup).includes("Mozsearch"), "The first visual line's tokens aren't in the replica");
  }

  // Clicking on a token shows its menu.
  {
    const popup = frame.contentDocument.querySelector("#blame-popup");
    const token = popup.querySelector(".rd-replica .hb-token");
    TestUtils.click(token);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "The context menu is shown");
    ok(menu.textContent.includes("Show the earliest version with this token"),
       "The context menu has the token's links");
    ok(!menu.textContent.includes("Select this token's line"),
       "Rendered documents don't have lines to select");
    frame.contentWindow.ContextMenu.hide();
  }
  frame.contentWindow.BlameStripHoverHandler.keepVisible = false;
  frame.contentWindow.BlamePopup.triggerElement = null;
});

add_task(async function test_RenderedLabels() {
  // References to Sphinx labels in other documents link (by the tree's
  // cross-reference) to the labels' lines in their documents' rendered views.
  await TestUtils.loadPath("/tests/rendered/docs/md_test/index.md");
  const refs = [...doc().querySelectorAll("a.rd-ref")];
  const label = refs.find(a => a.textContent == "rst-test-label");
  is(label?.getAttribute("href"), "/tests/rendered/docs/rst_test/index.rst#6",
     "A reference to another document's label links to the label's line");
  const title = refs.find(a => a.textContent == "the reST test");
  is(title?.getAttribute("href"), "/tests/rendered/docs/rst_test/index.rst#1",
     "Documents' titles are labels");
  ok(refs.find(a => a.getAttribute("href") == "#md-test-label"),
     "References to the document's own labels link to their anchors");

  // A label's line goes to the section it labels.
  await TestUtils.loadPath("/tests/rendered/docs/rst_test/index.rst#6");
  const target = doc().querySelector(".rd-hash-target");
  ok(target?.querySelector("h2"), "The line's block is highlighted");
  ok(target?.textContent.includes("Linking"), "The block is the section's heading");

  // In the documents' source listings, labels go to their documentation.
  await TestUtils.loadPath("/tests/source/docs/md_test/index.md");
  const use = frame.contentDocument.querySelector('[data-symbols="DOCLABEL_rst@2Dtest@2Dlabel"]');
  ok(use, "The reference to the label has its symbol");
  TestUtils.click(use);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "The context menu is shown");
  const item = [...menu.querySelectorAll("a")].find(
    a => a.textContent.includes("Go to documentation of"));
  is(item?.getAttribute("href"), "/tests/rendered/docs/rst_test/index.rst#6",
     "The menu goes to the label in its document's rendered view");
  frame.contentWindow.ContextMenu.hide();
});

add_task(async function test_RenderedTip() {
  // The tip's hyperblame data is the static files.
  await TestUtils.loadPath("/searchfox/rendered/README.md");
  ok(frame.contentWindow.BLAME_INFO.dataUrl.startsWith("/searchfox/hyperblame/"),
     "The tip uses the static hyperblame data");
  const strip = doc().querySelector(".blame-strip");
  const popup = await showPopup(strip, strip.getBoundingClientRect().top + 2);
  ok(replicaTokens(popup).length, "The popup shows tokens");
  frame.contentWindow.BlamePopup.triggerElement = null;
});
