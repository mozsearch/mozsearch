"use strict";

// Uses of prefs by name in JS, C++, and markup go to their definitions in
// prefs files and StaticPrefList.yaml.
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

add_task(async function test_prefs_js_use() {
  await checkGoToDefinition(
    "/tests/source/prefs/consumer.mjs",
    `#line-10 span[data-symbols="PREFS_test@2Ejs@2Epref"]`,
    "test.js.pref",
    "/tests/source/prefs/all.js", "#8");
});

add_task(async function test_prefs_cpp_use() {
  await checkGoToDefinition(
    "/tests/source/prefs/PrefsConsumer.cpp",
    `#line-22 span[data-symbols="PREFS_test@2Eint"]`,
    "test.int",
    "/tests/source/staticprefs/StaticPrefList.yaml", "#1");
});

add_task(async function test_prefs_markup_use() {
  await checkGoToDefinition(
    "/tests/source/prefs/preferences.xhtml",
    `#line-5 span[data-symbols="PREFS_test@2Ejs@2Epref"]`,
    "test.js.pref",
    "/tests/source/prefs/all.js", "#8");
});
