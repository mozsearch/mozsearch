"use strict";

add_task(async function test_BreadcrumbsContextMenu() {
  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  const breadcrumbs = frame.contentDocument.querySelector(".breadcrumbs");

  const menu = frame.contentDocument.querySelector("#context-menu");
  TestUtils.click(breadcrumbs);
  is(menu.style.display, "none",
     "Context menu is not opened");
});

add_task(async function test_DiagramControlToggleContextMenu() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  const toggle = frame.contentDocument.querySelector("#diagram-panel-toggle");

  const menu = frame.contentDocument.querySelector("#context-menu");
  TestUtils.click(toggle);
  is(menu.style.display, "none",
     "Context menu is not opened");
});

add_task(async function test_FileSymbolContextMenu() {
  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  const fileSymbol = frame.contentDocument.querySelector(".breadcrumbs [data-symbols]");
  is(fileSymbol.textContent, "(file symbol)",
     "File symbol exists inside the context menu");

  const menu = frame.contentDocument.querySelector("#context-menu");
  TestUtils.click(fileSymbol);
  await waitForShown(menu, "Context menu is shown");
});
