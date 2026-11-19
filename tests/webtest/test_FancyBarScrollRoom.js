"use strict";

// While the fancy bar is open, a page wider than the window (ex: a big block
// diagram) can be scrolled out from under it (panel.js's `updateScrollRoom`),
// and a page which fits gets no room, nor a scroll bar.
add_task(async function test_FancyBarScrollRoom() {
  registerCleanupFunction(() => {
    // (The bar's width the resizer kept for later pages.)
    frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("fancy-bar-width"));
  });
  await TestUtils.resetFeatureGate("fancyBar");
  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  const doc = frame.contentDocument;
  const win = frame.contentWindow;
  const root = doc.documentElement;
  const panel = doc.getElementById("panel");
  ok(root.classList.contains("fancy-bar-open"), "The fancy bar is open");
  const room = doc.querySelector(".fancy-bar-scroll-room");
  ok(room, "The fancy bar has its scroll room");

  await TestUtils.waitForCondition(() => room.hidden, "A page which fits has no scroll room");
  is(root.scrollWidth, root.clientWidth, "A page which fits doesn't scroll sideways");

  const wide = doc.createElement("div");
  wide.style.width = `${root.clientWidth * 2}px`;
  wide.style.height = "10px";
  doc.getElementById("scrolling").append(wide);
  await TestUtils.waitForCondition(() => !room.hidden, "A wider page has scroll room");

  win.scrollTo(root.scrollWidth, 0);
  const right = wide.getBoundingClientRect().right;
  const barLeft = panel.getBoundingClientRect().left;
  ok(right <= barLeft + 1, `The page's right end (${right}) scrolls out from under the bar (${barLeft})`);
  ok(right >= barLeft - 2, `It scrolls only as far as the bar's left (${right}, ${barLeft})`);

  // (A narrower bar, as its resizer's arrow keys make it.)
  const width = panel.getBoundingClientRect().width;
  const resizer = doc.querySelector(".fancy-bar-resizer");
  resizer.dispatchEvent(new win.KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  await TestUtils.waitForCondition(() => panel.getBoundingClientRect().width < width,
                                   "The bar is narrower");
  await TestUtils.waitForCondition(() => {
    win.scrollTo(root.scrollWidth, 0);
    return Math.abs(wide.getBoundingClientRect().right - panel.getBoundingClientRect().left) <= 2;
  }, "The room follows the bar's width");

  TestUtils.click(doc.getElementById("panel-toggle"));
  ok(!root.classList.contains("fancy-bar-open"), "The fancy bar is closed");
  await TestUtils.waitForCondition(() => room.hidden, "A closed bar has no scroll room");

  TestUtils.click(doc.getElementById("panel-toggle"));
  await TestUtils.waitForCondition(() => !room.hidden, "The reopened bar has scroll room");

  wide.remove();
  await TestUtils.waitForCondition(() => room.hidden, "The page fits again: no scroll room");
  is(root.scrollWidth, root.clientWidth, "Nor does it scroll sideways");
});
