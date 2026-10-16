"use strict";

// Symbols with commas (ex: a C++ partial specialization's members', with the
// template's arguments in their qualified names, or a JS property's with a
// string key) are escaped ("@2C"), since commas separate symbols in source
// records and pages' data-symbols attributes, so that their tokens' menus
// work: the clang plugin, merge-analyses, and js-analyze escape them.
async function checkSearch(path, lineText, tokenText, defHeading) {
  await TestUtils.loadPath(path);
  const line = [...frame.contentDocument.querySelectorAll(".source-line")]
    .find(node => node.textContent.includes(lineText));
  const token = [...line.querySelectorAll("[data-symbols]")].find(node => node.textContent == tokenText);
  // (Commas separate the token's symbols, which are whole: the page has their
  // jumps.)
  const syms = token?.dataset.symbols.split(",") ?? [];
  ok(syms.length && syms.every(sym => sym.includes("@2C") && frame.contentWindow.SYM_INFO[sym]),
     `${tokenText}'s symbols are whole (${token?.dataset.symbols})`);

  TestUtils.click(token);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");
  const search = menu.querySelector('a[href*="/search?q=symbol:"]');
  ok(search, "The menu has a search for its symbol");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(search);
  await loadPromise;
  await waitForCondition(() => frame.contentDocument.querySelector("#content").textContent.includes(defHeading),
                         `which finds its definition (${defHeading})`);
}

add_task(async function test_TemplateArgumentCommas() {
  // (A typedef's definition is an alias.)
  await checkSearch("/tests/source/cpp/template_commas.cpp", "using First = A;", "First", "Aliases");
});

add_task(async function test_StringKeyCommas() {
  await checkSearch("/tests/source/js/comma_keys.js", '"a,b": 1', "a,b", "Definitions");
});
