"use strict";

add_task(async function test_defineESModuleGetters() {
  const path = "/tests/source/js/lazyGetterAPI.js";
  await TestUtils.loadPath(path);

  const defineESModuleGetters = frame.contentDocument.querySelector('span[data-symbols*="#defineESModuleGetters"]');
  ok(!!defineESModuleGetters, "defineESModuleGetters token exists");

  TestUtils.click(defineESModuleGetters);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const typescriptRows = menu.querySelectorAll(".icon-export-alt");
  is(typescriptRows.length, 1, "Menu has 1 TypeScript row");
  ok(typescriptRows[0].classList.contains("submenu-label"), "The TypeScript row is a submenu");
  is(typescriptRows[0].textContent, "Possible TypeScript definitions");

  TestUtils.dispatchMouseEvent("mouseenter", typescriptRows[0]);

  await TestUtils.waitForCondition(() => frame.contentDocument.querySelector(".context-submenu"), "Submenu is eventually created on mouseenter");
  const submenu = frame.contentDocument.querySelector(".context-submenu");
  await waitForShown(submenu, "Submenu is eventually shown on mouseenter");

  const directJumps = submenu.querySelectorAll('a[href*="/tests/source"]');
  is(directJumps.length, 1, "1 direct jump is shown");

  is(directJumps[0].textContent, "Go to definition of ChromeUtils::defineESModuleGetters", "Direct jump names TypeScript symbol");
  ok(directJumps[0].href.includes("/tests/source/ts/src/ChromeUtils.js#"), "Direct jump links to TypeScript definition");

  const searches = submenu.querySelectorAll('a[href*="/tests/search"]');
  is(searches.length, 1, "1 search is shown");

  is(searches[0].textContent, "Search for ChromeUtils::defineESModuleGetters", "Search names TypeScript symbol");
  ok(searches[0].href.includes("/tests/search?q=symbol:S_js_ts%2Fsrc%2FChromeUtils.js%2FChromeUtils.defineESModuleGetters()."), "Search links to TypeScript symbol");
});
