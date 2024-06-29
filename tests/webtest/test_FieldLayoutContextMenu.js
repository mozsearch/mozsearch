"use strict";

function findClassLayoutMenuItem(menu) {
  for (const row of menu.querySelectorAll(".contextmenu-row")) {
    if (row.textContent.startsWith("Class layout of ")) {
      return row;
    }
  }

  return null;
}

add_task(async function test_FieldLayoutContextMenu() {
  // Enabled by default on the test config.
  await TestUtils.resetFeatureGate("semanticInfo");
  await TestUtils.loadPath("/tests/source/field-layout/field-type.cpp");

  const className = frame.contentDocument.querySelector(`span.syn_def[data-symbols="T_field_layout::field_type::S"]`);
  TestUtils.click(className);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  let layoutRow = findClassLayoutMenuItem(menu);
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

add_task(async function test_FieldLayoutContextMenu_gate() {
  registerCleanupFunction(async () => {
    await TestUtils.resetFeatureGate("semanticInfo");
  });

  const tests = [
    { value: "release", shown: false },
    { value: "beta", shown: false },
    { value: "alpha", shown: true },
  ];
  for (const { value, shown } of tests) {
    await TestUtils.setFeatureGate("semanticInfo", value);
    await TestUtils.loadPath("/tests/source/field-layout/field-type.cpp");

    const className = frame.contentDocument.querySelector(`span.syn_def[data-symbols="T_field_layout::field_type::S"]`);
    TestUtils.click(className);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    let layoutRow = findClassLayoutMenuItem(menu);
    if (shown) {
      ok(!!layoutRow, `Class layout menu item exists on ${value}`);
    } else {
      ok(!layoutRow, `Class layout menu item does not exist on ${value}`);
    }
  }
});
