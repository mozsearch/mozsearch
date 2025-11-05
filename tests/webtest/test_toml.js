"use strict";

function findMenuItem(menu, text) {
  for (const row of menu.querySelectorAll(".contextmenu-row")) {
    if (row.textContent.includes(text)) {
      return row;
    }
  }

  return null;
}

add_task(async function test_toml() {
  await TestUtils.loadPath("/tests/source/tests/mochitest.toml");

  const tests = [
    {
      line: 2,
      path: "tests/head.js",
    },
    {
      line: 4,
      path: "tests/support.html",
    },
    {
      line: 6,
      path: "tests/support.txt",
      missing: true,
    },
    {
      line: 8,
      path: "js/export.mjs",
    },
    {
      line: 13,
      path: "tests/file_something.html",
    },
    {
      line: 26,
      path: "tests/mochitest-common.toml",
    },
  ];

  for (const test of tests) {
    const call = frame.contentDocument.querySelector(`#line-${test.line} .syn_string`);
    TestUtils.click(call);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const row = findMenuItem(menu, `Go to definition of ${test.path}`);
    if (test.missing) {
      ok(!row, `The menu item for ${test.path} does not exist`);
    } else {
      ok(row, `The menu item for ${test.path} exists`);
    }
  }
});
