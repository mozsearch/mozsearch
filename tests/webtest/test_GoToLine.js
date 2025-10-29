"use strict";

add_task(async function test_GoToLineMenu() {
  await TestUtils.loadPath("/tests/source/big_cpp.cpp");

  // Click a "Thing" class consumer, places far after the definition.
  const ThingUse = frame.contentDocument.querySelector("#line-414 .syn_type");
  ThingUse.scrollIntoView();

  const ThingDef = frame.contentDocument.querySelector("#line-136 .syn_def");

  await waitForCondition(() => ThingDef.getBoundingClientRect().y < 0,
                         "The definition should not be visible at this point");

  TestUtils.click(ThingUse);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol");

  const linkRows = menu.querySelectorAll(".contextmenu-link");
  let gotoRow;
  for (const row of linkRows) {
    console.log("[" + row.textContent + "]");
    if (row.textContent == "Go to definition of outerNS::Thing") {
      gotoRow = row;
      break;
    }
  }
  ok(gotoRow, "Go to link is found");

  const oldRoot = frame.contentDocument.documentElement;

  TestUtils.click(gotoRow);

  await waitForCondition(() => ThingDef.getBoundingClientRect().y >= 0,
                         "The definition should become visible");

  is(oldRoot,frame.contentDocument.documentElement,
     "Should be in the same document");
});
