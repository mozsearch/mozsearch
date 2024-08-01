"use strict";

function goToDefinitionMenuItems(menu) {
  const items = []

  for (const row of menu.querySelectorAll(".contextmenu-row")) {
    if (row.textContent.startsWith("Go to definition of ")) {
      items.push(row)
    }
  }

  return items;
}

add_task(async function test_OverloadedFunctionInTemplateContextMenuHasMultipleDefs() {
  await TestUtils.loadPath("/tests/source/templates6.cpp");

  const overloaded = frame.contentDocument.querySelector("span[data-symbols*=overloaded]");
  TestUtils.click(overloaded);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown");

  const symbols = overloaded.dataset.symbols.split(',');
  is(symbols.length, 2, "2 symbols are available");

  const goToRows = goToDefinitionMenuItems(menu);
  is(goToRows.length, 2, "2 go to rows are available");
});
