"use strict";

// Uses of Fluent messages and resources in markup and JS go to their
// definitions in .ftl files.
async function checkGoToDefinition(path, selector, label, defPath, defHash) {
  await TestUtils.loadPath(path);

  const span = frame.contentDocument.querySelector(selector);
  ok(span, `${selector} is a symbol`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes(`Go to definition of ${label}`));
  ok(gotoDef, `A menu item to jump to the definition of ${label} exists`);

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname, defPath,
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash, defHash,
     "Jumps to the definition line");
}

add_task(async function test_fluent_markup_message() {
  await checkGoToDefinition(
    "/tests/source/fluent/fluent.html",
    `#line-9 span[data-symbols="FTL_tabs-close-button"]`,
    "tabs-close-button",
    "/tests/source/fluent/test.ftl", "#19");
});

add_task(async function test_fluent_js_message() {
  await checkGoToDefinition(
    "/tests/source/fluent/fluent.mjs",
    `#line-12 span[data-symbols="FTL_tabs-close-warning"]`,
    "tabs-close-warning",
    "/tests/source/fluent/test.ftl", "#12");
});

add_task(async function test_fluent_resource() {
  await checkGoToDefinition(
    "/tests/source/fluent/fluent.html",
    `#line-5 span[data-symbols="FILE_fluent/test@2Eftl"]`,
    "fluent/test.ftl",
    "/tests/source/fluent/test.ftl", "#1");
});
