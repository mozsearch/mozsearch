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

// A WebIDL method's spec definition's steps (ex: "The {{Gadget/poke()}}
// method steps are to ...", see `spec_tokenizer::idl_steps`) are an
// algorithm, which its context menu shows beside the code citing it.
add_task(async function test_webidl_method_steps() {
  const links = await menuLinks("/tests/source/webidl/gadget.webidl", "WEBIDL_Gadget_poke");
  const beside = links.find(a => a.textContent.includes("beside the code citing it"));
  is(beside?.getAttribute("href"), "/tests/algorithm/SPEC_examplebs_dom@2Dgadget@2Dpoke",
     "The WebIDL method's spec steps can be shown beside the code citing them");
  await TestUtils.loadPath("/tests/algorithm/SPEC_examplebs_dom@2Dgadget@2Dpoke");
  ok(frame.contentDocument.body.textContent.includes("method steps are to"),
     "which shows the method's steps");
});

// The steps' view has the WebIDL members reflecting the spec's definition (as
// BCD says: the test BCD data says BindingTest.method1 does, for its C++
// implementation in the test tree), each followed by its C++ implementations.
add_task(async function test_webidl_implementations() {
  await TestUtils.loadPath("/tests/algorithm/SPEC_examplebs_dom@2Dgadget@2Dpoke");
  const cards = [...frame.contentDocument.querySelectorAll(".sp-citation")];
  const webidl = cards.find(c => c.querySelector(".sp-citation-fn")?.textContent === "BindingTest::method1");
  is(webidl?.querySelector(".sp-citation-note")?.textContent, "WebIDL",
     "The WebIDL member reflecting the definition is beside its steps");
  const impl = cards[cards.indexOf(webidl) + 1];
  is(impl?.querySelector(".sp-citation-note")?.textContent,
     "C++ implementation of the WebIDL BindingTest::method1",
     "followed by its C++ implementation");
  ok(impl?.querySelector(".sp-citation-code")?.textContent.includes("Method1"),
     "with its code");
});
