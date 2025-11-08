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
  await TestUtils.loadPath("/tests/source/mozbuild/moz.build");

  const tests = [
    {
      line: 1,
      path: "mozbuild/other.mozbuild",
    },
    {
      line: 4,
      path: "mozbuild/sub",
    },
    {
      line: 9,
      path: "some_python.py",
    },
    {
      line: 10,
      path: "mozbuild/other.py",
    },
    {
      line: 14,
      path: "mozbuild/test.cpp",
    },
    {
      line: 16,
      path: "mozbuild/test.cpp",
    },
    {
      line: 19,
      path: "mozbuild/unified.cpp",
    },
    {
      line: 27,
      path: "__GENERATED__/__win64__/mozbuild/generated-header.h",
    },
    {
      line: 27,
      path: "__GENERATED__/__linux64__/mozbuild/generated-header.h",
    },
    {
      line: 32,
      path: "mozbuild/generated-header2.h",
      missing: true,
    },
    {
      line: 33,
      path: "atom_list.h",
    },
    {
      line: 39,
      path: "mozbuild/sub/test.js",
    },
    {
      line: 40,
      path: "mozbuild/test.css",
    },
    {
      line: 43,
      path: "__GENERATED__/__win64__/mozbuild/generated-header2.h",
    },
    {
      line: 43,
      path: "__GENERATED__/__linux64__/mozbuild/generated-header2.h",
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
