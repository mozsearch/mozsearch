"use strict";

function findMenuItemsForName(menu, name) {
  const items = [];
  for (const item of menu.querySelectorAll("a")) {
    if (item.textContent.includes("the substring")) {
      // The substring search is not target.
      continue;
    }
    if (item.textContent.includes(name)) {
      items.push(item);
    }
  }
  return items;
}

function getSearchHref(sym) {
  return `/tests/search?q=symbol:${encodeURIComponent(sym)}&redirect=false`;
}

add_task(async function test_TestModuleGlobals1() {
  const path1 = "/tests/source/js/module-global1.mjs";
  await TestUtils.loadPath(path1);

  const tests = [
    {
      line: 3,
      name: "ModuleGlobalTest_global_unique",
      sym: "#M-ModuleGlobalTest_global_unique",
      def: null,
    },
    {
      line: 4,
      name: "ModuleGlobalTest_global_conflict",
      sym: "#M-ModuleGlobalTest_global_conflict",
      def: null,
    },
    {
      line: 5,
      name: "ModuleGlobalTest_exported",
      sym: "#ModuleGlobalTest_exported",
      def: null,
    },
    {
      line: 6,
      name: "ModuleGlobalTest_exported_and_imported",
      sym: "#ModuleGlobalTest_exported_and_imported",
      def: null,
    },
    {
      line: 7,
      name: "ModuleGlobalTest_exported_and_global",
      sym: "#ModuleGlobalTest_exported_and_global",
      def: null,
    },
    {
      line: 8,
      name: "ModuleGlobalTest_exported_and_reexported",
      sym: "#ModuleGlobalTest_exported_and_reexported",
      def: null,
    },
    {
      line: 11,
      name: "ModuleGlobalTest_global_unique",
      sym: "#M-ModuleGlobalTest_global_unique",
      def: `${path1}#3`,
    },
    {
      line: 12,
      name: "ModuleGlobalTest_global_conflict",
      sym: "#M-ModuleGlobalTest_global_conflict",
      def: `${path1}#4`,
    },
    {
      line: 13,
      name: "ModuleGlobalTest_exported",
      sym: "#ModuleGlobalTest_exported",
      def: `${path1}#5`,
    },
    {
      line: 14,
      name: "ModuleGlobalTest_exported_and_imported",
      sym: "#ModuleGlobalTest_exported_and_imported",
      def: `${path1}#6`,
    },
    {
      line: 15,
      name: "ModuleGlobalTest_exported_and_global",
      sym: "#ModuleGlobalTest_exported_and_global",
      def: `${path1}#7`,
    },
    {
      line: 16,
      name: "ModuleGlobalTest_exported_and_reexported",
      sym: "#ModuleGlobalTest_exported_and_reexported",
      def: `${path1}#8`,
    },
  ];

  for (const test of tests) {
    const call = frame.contentDocument.querySelector(`#line-${test.line} [data-symbols]`);
    TestUtils.click(call);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const items = findMenuItemsForName(menu, test.name);
    if (test.def) {
      is(items.length, 2, `Context menu has expected items for line ${test.line}`);
      is(items[0].getAttribute("href"), test.def,
         `Go to definition should link to ${test.def} for line ${test.line}`);
      is(items[1].getAttribute("href"), getSearchHref(test.sym),
         `Search should link to ${test.sym} for line ${test.line}`);
    } else {
      is(items.length, 1, `Context menu has expected items for line ${test.line}`);
      is(items[0].getAttribute("href"), getSearchHref(test.sym),
         `Search should link to ${test.sym} for line ${test.line}`);
    }
  }
});

add_task(async function test_TestModuleGlobals2() {
  const path1 = "/tests/source/js/module-global1.mjs";
  const path2 = "/tests/source/js/module-global2.mjs";
  await TestUtils.loadPath(path2);

  const tests = [
    {
      line: 4,
      name: "ModuleGlobalTest_global_conflict",
      sym: "#M-ModuleGlobalTest_global_conflict",
      def: null,
    },
    {
      // Imported symbol should point the canonical definition,
      // even inside the import declaration.
      line: 6,
      name: "ModuleGlobalTest_exported_and_imported",
      sym: "#ModuleGlobalTest_exported_and_imported",
      def: `${path1}#6`,
    },
    {
      line: 7,
      name: "ModuleGlobalTest_exported_and_global",
      sym: "#M-ModuleGlobalTest_exported_and_global",
      def: null,
    },
    {
      // Re-exported symbol should point the canonical definition.
      line: 8,
      name: "ModuleGlobalTest_exported_and_reexported",
      sym: "#ModuleGlobalTest_exported_and_reexported",
      def: `${path1}#8`,
    },
    {
      line: 12,
      name: "ModuleGlobalTest_global_conflict",
      sym: "#M-ModuleGlobalTest_global_conflict",
      def: `${path2}#4`,
    },
    {
      line: 14,
      name: "ModuleGlobalTest_exported_and_imported",
      sym: "#ModuleGlobalTest_exported_and_imported",
      def: `${path1}#6`,
    },
    {
      line: 15,
      name: "ModuleGlobalTest_exported_and_global",
      sym: "#M-ModuleGlobalTest_exported_and_global",
      def: `${path2}#7`,
    },
    {
      line: 16,
      name: "ModuleGlobalTest_exported_and_reexported",
      sym: "#ModuleGlobalTest_exported_and_reexported",
      def: `${path1}#8`,
    },
  ];

  for (const test of tests) {
    const call = frame.contentDocument.querySelector(`#line-${test.line} [data-symbols]`);
    TestUtils.click(call);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const items = findMenuItemsForName(menu, test.name);
    if (test.def) {
      is(items.length, 2, `Context menu has expected items for line ${test.line}`);
      is(items[0].getAttribute("href"), test.def,
         `Go to definition should link to ${test.def} for line ${test.line}`);
      is(items[1].getAttribute("href"), getSearchHref(test.sym),
         `Search should link to ${test.sym} for line ${test.line}`);
    } else {
      is(items.length, 1, `Context menu has expected items for line ${test.line}`);
      is(items[0].getAttribute("href"), getSearchHref(test.sym),
         `Search should link to ${test.sym} for line ${test.line}`);
    }
  }
});

add_task(async function test_ModuleGlobalAndThenExport() {
  const path = "/tests/source/js/export9.mjs";
  await TestUtils.loadPath(path);

  const call = frame.contentDocument.querySelector(`#line-3 [data-symbols]`);
  TestUtils.click(call);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const items = findMenuItemsForName(menu, "declaredAsLocalAndThenExported");
  is(items.length, 3, `Context menu has expected items`);
  is(items[0].getAttribute("href"), `${path}#1`,
     `Go to definition should link to the module global.`);
  is(items[1].getAttribute("href"), getSearchHref(`#M-declaredAsLocalAndThenExported`),
     `The first search should be for the module global`);
  ok(items[1].textContent.includes("module global"),
     `The first search should be for the module global`);
  is(items[2].getAttribute("href"), getSearchHref(`#declaredAsLocalAndThenExported`),
     `The second search should be for the exported symbol`);
  ok(!items[2].textContent.includes("module global"),
     `The second search should be for the exported symbol`);
});
