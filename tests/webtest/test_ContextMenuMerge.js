"use strict";

add_task(async function test_TreeSwitcherKeyboardNavigation() {
  await TestUtils.loadPath("/tests/source/cpp/context-menu-search-merge.cpp");

  const call = frame.contentDocument.querySelector("#line-61 .source-line span");
  TestUtils.click(call);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  // There are 4 different definitions, where:
  //   * first 3 share the source line with different class
  //   * last 1 is in different source line with different class
  const searchRows = menu.querySelectorAll(".icon-search");
  is(searchRows.length, 2, "2 search items are visible");

  ok(searchRows[0].href.includes("_ZN12search_merge4funcERKNS_3ns11CE"),
     "ns1 is in the first item");
  ok(searchRows[0].href.includes("_ZN12search_merge4funcERKNS_3ns21CE"),
     "ns2 is in the first item");
  ok(searchRows[0].href.includes("_ZN12search_merge4funcERKNS_3ns31CE"),
     "ns3 is in the first item");
  ok(searchRows[1].href.includes("_ZN12search_merge4funcERKNS_3ns41CE"),
     "ns4 is in the second item");
});
