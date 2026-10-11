"use strict";

// Citations of web specs (see tools/src/spec_analyze/citations.rs) go to the
// specs' definitions: in the tests tree, whose specs they are, and in the
// searchfox tree, whose `SPEC_` symbols are the tests tree's (its
// `linked_trees`), in the tests tree's files.
async function checkCitation(path, defPath, defHash) {
  await TestUtils.loadPath(path);

  const selector = `#line-11 span[data-symbols="SPEC_example_concept@2Dwidget@2Dpoke"]`;
  const span = frame.contentDocument.querySelector(selector);
  ok(span, "The citation is a symbol");

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the citation");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to definition of example poke"));
  ok(gotoDef, "A menu item to jump to the spec's definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname, defPath,
     "Jumps to the spec's source");
  is(frame.contentDocument.location.hash, defHash,
     "Jumps to the definition's line");
}

add_task(async function test_citation_in_its_specs_tree() {
  await checkCitation(
    "/tests/source/spec_citations.cpp",
    "/tests/source/specs/source", "#13");
});

add_task(async function test_citation_in_a_linked_tree() {
  await checkCitation(
    "/searchfox/source/tests/tests/files/spec_citations.cpp",
    "/tests/source/specs/source", "#13");
});

// Searching a linked tree for a spec's symbol has the tests tree's results
// (the spec's definition and uses) in their own section, linking to its files.
add_task(async function test_linked_search_results() {
  await TestUtils.loadPath(
    "/searchfox/search?q=symbol:SPEC_example_concept@2Dwidget@2Dpoke&redirect=false");
  TestUtils.shortenSearchTimeouts();

  const content = frame.contentDocument.querySelector("#content");
  await waitForCondition(
    () => content.textContent.includes("In tests"),
    "The tests tree's results have their own section"
  );

  const links = [...content.querySelectorAll("a")].map(a => new URL(a.href).pathname + new URL(a.href).hash);
  ok(links.includes("/tests/source/specs/source#13"),
     "The linked tree's definition links to its file");
  ok(links.some(link => link.startsWith("/searchfox/source/tests/tests/files/spec_citations.cpp#")),
     "The tree's own citations link to its files");
});
