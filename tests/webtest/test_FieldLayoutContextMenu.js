"use strict";

add_task(async function test_FieldLayoutContextMenu() {
  await TestUtils.loadPath("/tests/source/field-layout/field-type.cpp");

  const className = frame.contentDocument.querySelector(`span.syn_def[data-symbols="T_field_layout::field_type::S"]`);
  TestUtils.click(className);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  let layoutRow = null;
  for (const row of menu.querySelectorAll(".contextmenu-row")) {
    if (row.textContent.startsWith("Class layout of ")) {
      layoutRow = row;
      break;
    }
  }
  ok(!!layoutRow, "Class layout menu item exists");
  is(layoutRow.textContent, "Class layout of field_layout::field_type::S",
     "Menu item shows the qualified class name");

  const loadPromise = TestUtils.waitForLoad();
  const link = layoutRow.querySelector(".contextmenu-link");
  TestUtils.click(link);
  await loadPromise;
  ok(frame.contentDocument.location.href.includes("/query/"),
     "Navigated to query page");

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "field-layout:'field_layout::field_type::S'",
     "Query for field layout is set");
});
