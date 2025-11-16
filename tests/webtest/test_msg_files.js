"use strict";

add_task(async function test_msg_MacroExpansions() {
  // Enabled by default on the test config.
  await TestUtils.resetFeatureGate("expansions");
  await TestUtils.loadPath("/tests/source/cpp/errors.msg");

  const msg_def = frame.contentDocument.querySelector("span[data-expansions]");
  TestUtils.click(msg_def);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for macro click");

  const expansionRows = menu.querySelectorAll(".contextmenu-expansion-preview");
  is(expansionRows.length, 2, "2 expansion rows are visible");
});
