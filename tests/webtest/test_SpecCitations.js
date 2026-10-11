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
