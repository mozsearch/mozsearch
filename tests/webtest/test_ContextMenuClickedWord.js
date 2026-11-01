"use strict";

// The context menu's text search is for the word clicked on, at the click's
// point (see context-menu.js's `getTargetWord`), not for the selection's
// word, which a click elsewhere than on text (ex: on a diagram's background,
// or a line's past its end, or a breadcrumb's button) leaves where an
// earlier click put it.

function doc() {
  return frame.contentDocument;
}

// Clicks at the point of the frame's viewport (x, y), as a mouse does (its
// position and count), on what's there.
function clickAt(x, y) {
  const target = doc().elementFromPoint(x, y);
  target.dispatchEvent(new MouseEvent("click", {
    bubbles: true, cancelable: true, clientX: x, clientY: y, detail: 1,
  }));
  return target;
}

// The context menu's items, if it's shown, which it then hides.
function takeMenu() {
  const menu = doc().querySelector("#context-menu");
  const items = menu && menu.style.display != "none"
    ? [...menu.querySelectorAll("a")].map(a => a.textContent)
    : [];
  doc().body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  return items;
}

// The text node in `elem` with `word`, and the middle of the word's box.
function wordPoint(elem, word) {
  const walker = doc().createTreeWalker(elem, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const at = walker.currentNode.nodeValue.indexOf(word);
    if (at < 0) {
      continue;
    }
    const range = doc().createRange();
    range.setStart(walker.currentNode, at);
    range.setEnd(walker.currentNode, at + word.length);
    const rect = range.getBoundingClientRect();
    return { node: walker.currentNode, at, x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
  }
  return null;
}

// (As an earlier click on `node` at `at` would leave the selection.)
function caretAt(node, at) {
  doc().getSelection().collapse(node, at);
}

const searches = items => items.filter(item => item.startsWith("Search for the substring"));

add_task(async function test_ClickedWord() {
  await TestUtils.loadPath("/tests/source/bug2063622.cpp");
  const line = doc().querySelector("#line-8 code");
  line.scrollIntoView({ block: "center" });
  const enough = wordPoint(line, "enough");
  ok(enough, "The line has the word");

  // A click on a word searches for it.
  clickAt(enough.x, enough.y);
  is(JSON.stringify(searches(takeMenu())), JSON.stringify(["Search for the substring enough"]),
     "A click on a word searches for it");

  // A click past the line's end doesn't search for the word the selection's
  // in.
  caretAt(enough.node, enough.at + 2);
  const end = wordPoint(line, "required)");
  const range = doc().createRange();
  range.setStart(end.node, end.at);
  range.setEnd(end.node, end.at + "required)".length);
  ok(clickAt(range.getBoundingClientRect().right + 30, enough.y).closest("code"),
     "(The click is on the line, past its text)");
  is(JSON.stringify(searches(takeMenu())), "[]", "A click on the line past its end searches for nothing");
});

add_task(async function test_DiagramBackground() {
  await TestUtils.loadQuery("tests", "block-diagram:'Core/Rollup View'");
  const svg = doc().querySelector("svg");
  svg.scrollIntoView({ block: "center" });
  const label = [...svg.querySelectorAll("text")].find(text => text.textContent == "Panel");
  const panel = wordPoint(label, "Panel");
  clickAt(panel.x, panel.y);
  ok(searches(takeMenu()).includes("Search for the substring Panel"), "A click on a node's label searches for it");

  // (Its background, where the click put the selection earlier.)
  caretAt(panel.node, 2);
  const box = svg.getBoundingClientRect();
  const target = clickAt(box.left + 3,
                         (Math.max(box.top, 0) + Math.min(box.bottom, frame.contentWindow.innerHeight)) / 2);
  ok(svg.contains(target) && !target.closest("text"), `(The click is on the diagram's background, a ${target.tagName})`);
  is(JSON.stringify(takeMenu()), "[]", "A click on a diagram's background shows no menu");
});

add_task(async function test_BreadcrumbButton() {
  await TestUtils.loadQuery("searchfox", "crash-report:5e4c0b1a-7e57-4c0d-8e5a-000000261101");
  // (The tree's breadcrumb, where a click put the selection earlier.)
  const tree = doc().querySelector(".breadcrumbs a");
  caretAt(tree.firstChild, 3);
  const button = doc().getElementById("crash-report-menu-button");
  const rect = button.getBoundingClientRect();
  clickAt((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
  is(doc().getElementById("crash-report-menu").style.display, "flex", "A breadcrumb's menu button shows its menu");
  is(JSON.stringify(takeMenu()), "[]", "but not the context menu");
});
