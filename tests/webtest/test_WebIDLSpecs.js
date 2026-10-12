"use strict";

// The tree's WebIDL symbols link to the specs' definitions which they reflect
// (their `spec` binding slots), and to their MDN pages, as MDN's
// browser-compat-data has them (see tools/src/spec_analyze/bcd.rs and the test
// BCD data in tests/tests/bcd/data.json).

async function menuLinks(path, sym) {
  await TestUtils.loadPath(path);
  const span = frame.contentDocument.querySelector(`span[data-symbols="${sym}"]`);
  ok(span, `${sym} is in ${path}`);
  TestUtils.click(span);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown");
  return [...menu.querySelectorAll("a")];
}

add_task(async function test_webidl_spec_and_mdn() {
  const links = await menuLinks("/tests/source/webidl/gadget.webidl", "WEBIDL_Gadget_poke");
  const spec = links.find(a => a.textContent.includes("IDL spec method"));
  ok(spec?.textContent.includes("examplebs Gadget/poke"),
     "The WebIDL method goes to the spec's definition");
  ok(spec?.getAttribute("href").startsWith("/tests/source/specs/example.bs#"),
     "in the spec's source");
  const mdn = links.filter(a => a.textContent.includes("MDN documentation"));
  is(mdn.length, 1, "It has its MDN page, once (the spec's definition has it too)");
  is(mdn[0]?.getAttribute("href"), "https://developer.mozilla.org/docs/Web/API/Gadget/poke",
     "linking it");
});

add_task(async function test_spec_definition_mdn() {
  const links = await menuLinks("/tests/source/specs/source", "SPEC_example_concept@2Dwidget@2Dstate");
  const mdn = links.find(a => a.textContent.includes("MDN documentation"));
  is(mdn?.getAttribute("href"), "https://developer.mozilla.org/docs/Web/CSS/widget-state",
     "A spec's definition links to its MDN page");
});
