"use strict";

function findGCMenuItem(menu) {
  for (const row of menu.querySelectorAll(".contextmenu-row")) {
    if (row.textContent.match("Can GC|Cannot GC")) {
      return row;
    }
  }

  return null;
}

add_task(async function test_CanGCContextMenu() {
  // Enabled by default on the test config.
  await TestUtils.resetFeatureGate("semanticInfo");
  await TestUtils.loadPath("/tests/source/cpp/gc.cpp");

  {
    const sym = frame.contentDocument.querySelector("span[data-symbols=_ZN2GC6CanGC2Ev]");
    TestUtils.click(sym);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown");

    const gcRow = findGCMenuItem(menu);
    ok(!!gcRow, "GC menu item exists");
    is(gcRow.textContent, "Can GC",
       "CanGC2 can GC");

    const link = gcRow.querySelector(".contextmenu-link");
    TestUtils.click(link);

    const blamePopup = frame.contentWindow.BlamePopup;
    const blameStripHoverHandler = frame.contentWindow.BlameStripHoverHandler;
    await waitForShown(blamePopup.popup, "BlamePopup is shown");

    const code = blamePopup.popup.querySelector("code");
    is(code.textContent.trim(),
       "> bool CanGC(int foo)\n" + "> void DoGC()\n" + "> (GC)",
       "Call path should be shown");
  }

  {
    const sym = frame.contentDocument.querySelector("span[data-symbols=_ZN2GC8CannotGCEv]");
    TestUtils.click(sym);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown");

    const gcRow = findGCMenuItem(menu);
    ok(!!gcRow, "GC menu item exists");
    is(gcRow.textContent, "Cannot GC",
       "CannotGC cannot GC");

    const link = gcRow.querySelector(".contextmenu-link");
    TestUtils.click(link);

    const blamePopup = frame.contentWindow.BlamePopup;
    const blameStripHoverHandler = frame.contentWindow.BlameStripHoverHandler;
    await waitForShown(blamePopup.popup, "BlamePopup is shown");

    console.log(blamePopup.popup.textContent);
    is(blamePopup.popup.textContent,
       "This function cannot GC.",
       "Message should be shown for Cannot GC case");
  }
});
