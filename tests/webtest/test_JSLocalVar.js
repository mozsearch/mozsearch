"use strict";

add_task(async function test_JSLocalVarContextMenu() {
  await TestUtils.loadPath("/tests/source/js/local_vars.js");

  const use = frame.contentDocument.querySelector(`#line-8 [data-symbols]`);

  TestUtils.click(use);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const gotoRow = menu.querySelector(".icon-export-alt");
  is(gotoRow.textContent, "Go to definition of localVariable",
     "Jump menu is shown for local variable");

  TestUtils.click(gotoRow);

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("#4"),
    "Jumps to the definition");
});
