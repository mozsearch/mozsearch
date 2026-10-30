"use strict";

// Citations of web specs (see tools/src/spec_analyze/citations.rs) go to the
// specs' definitions: in the tests tree, whose specs they are, and in the
// searchfox tree, whose `SPEC_` symbols are the tests tree's (its
// `linked_trees`), in the tests tree's files.
async function checkCitation(path, defPath, defHash, {
  line = 11,
  sym = "SPEC_example_concept@2Dwidget@2Dpoke",
  pretty = "example poke",
} = {}) {
  await TestUtils.loadPath(path);

  const selector = `#line-${line} span[data-symbols="${sym}"]`;
  const span = frame.contentDocument.querySelector(selector);
  ok(span, "The citation is a symbol");

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the citation");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes(`Go to definition of ${pretty}`));
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

// A citation by an outdated URL (one of the spec's `old_urls`) goes to the
// definition too, but is marked, and its menu says it's outdated, with the
// definition's current URL.
add_task(async function test_outdated_citation() {
  await TestUtils.loadPath("/tests/source/spec_citations.cpp");
  const span = frame.contentDocument.querySelector(
    `#line-59 span[data-symbols="SPEC_example_concept@2Dwidget"]`);
  ok(span?.classList.contains("syn_outdated"),
     "The citation is marked as by an outdated URL");

  TestUtils.click(span);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the citation");
  const links = [...menu.querySelectorAll("a")];
  ok(links.find(a => a.textContent.includes("Go to definition of")),
     "It goes to the definition");
  const note = links.find(a => a.textContent.includes("This cites an outdated URL"));
  is(note?.getAttribute("href"), "https://example.spec.whatwg.org/#concept-widget",
     "and says it's outdated, with its current URL");
});

// A citation of the id Wattsi generates for a cross-reference goes to the
// cross-reference.
add_task(async function test_cross_reference_citation() {
  await checkCitation(
    "/searchfox/source/tests/tests/files/spec_citations.cpp",
    "/tests/source/specs/source", "#38", {
      line: 45,
      sym: "SPEC_example_gadgets@3Aconcept@2Dwidget@2D2",
      pretty: "example #gadgets:concept-widget-2",
    });
});

// A citation of an anchor its spec (example-2) doesn't define, which another
// spec of its series (the example spec) does, is of that spec's, in a linked
// tree too, by the linked tree's crossref.
add_task(async function test_series_fallback_citation() {
  await checkCitation(
    "/searchfox/source/tests/tests/files/spec_citations.cpp",
    "/tests/source/specs/source", "#9", {
      line: 52,
      sym: "SPEC_example_concept@2Dwidget",
      pretty: "example widget",
    });
});

// A citation of an algorithm can show the algorithm beside the code citing it
// (see test_RenderedSpecs.js), in its spec's tree.
add_task(async function test_citation_algorithm_view() {
  await TestUtils.loadPath("/searchfox/source/tests/tests/files/spec_citations.cpp");
  const span = frame.contentDocument.querySelector(
    '#line-11 span[data-symbols="SPEC_example_concept@2Dwidget@2Dpoke"]');
  TestUtils.click(span);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the citation");
  const item = [...menu.querySelectorAll("a")]
    .find(a => a.textContent.includes("example poke beside the code citing it"));
  is(item?.getAttribute("href"), "/tests/algorithm/SPEC_example_concept@2Dwidget@2Dpoke",
     "The menu shows the algorithm beside the code citing it, in its spec's tree");
  frame.contentWindow.ContextMenu.hide();
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

// The query page's results for a spec's symbol in a linked tree likewise have
// the tests tree's results in their own section, linking to its files.
add_task(async function test_linked_query_results() {
  await TestUtils.loadPath(
    "/searchfox/query/default?q=symbol:SPEC_example_concept@2Dwidget@2Dpoke");

  const content = frame.contentDocument.querySelector("#content") || frame.contentDocument.body;
  await waitForCondition(
    () => content.textContent.includes("In tests"),
    "The tests tree's results have their own section"
  );

  const links = [...content.querySelectorAll("a")].map(a => new URL(a.href).pathname + new URL(a.href).hash);
  ok(links.includes("/tests/source/specs/source"),
     "The linked tree's file links to its file");
  // (A spec's source's lines go to its rendered view; see
  // test_SpecResultLinks.)
  ok(links.includes("/tests/rendered/specs/source/line/13"),
     "The linked tree's definition's line links to its line, in its spec's rendered view");
});

// The calls-to diagram of the example spec's poke algorithm in the searchfox
// tree has its implementations: the tree's own (citations.rs's test citing
// it), and the linked tests tree's (spec_citations.cpp's Poke), whose
// symbol's data is that tree's, with its jumps to its files.
add_task(async function test_linked_implementations_diagram() {
  await TestUtils.loadPath(
    "/searchfox/query/default?q=calls-to-sym:SPEC_example_concept@2Dwidget@2Dpoke%20depth:2");

  const content = frame.contentDocument.body;
  await waitForCondition(
    () => content.querySelector("svg") && content.textContent.includes("test_example_spec_citation"),
    "The diagram has the tree's implementation"
  );
  ok(content.textContent.includes("spec_citations::Poke"),
     "The diagram has the linked tree's implementation");

  const symInfo = frame.contentWindow.SYM_INFO;
  const linkedSym = "_ZN14spec_citations4PokeERNS_6WidgetE";
  ok(symInfo[linkedSym], "The linked tree's implementation has symbol info");
  is(symInfo[linkedSym]?.tree, "tests", "Its jumps are to the linked tree's files");
});

// A diagram's traversal reaches a linked tree's symbols (from the searchfox
// tree, whose spec symbols are the tests tree's: the tests tree's code
// citing the example spec's poke), unless a `-tree:` term leaves the tree
// out, or a `tree:` term doesn't list it (see the tools' `Traverse::trees`).
add_task(async function test_linked_tree_terms() {
  const QUERY = "calls-to-sym:'SPEC_example_concept@2Dwidget@2Dpoke' depth:2 graph-format:mozsearch-interactive";
  const linked = async query => {
    await TestUtils.loadQuery("searchfox", query);
    const win = frame.contentWindow;
    // (Not the spec's symbol itself, whose definition is the tests tree's too.)
    return (win.GRAPH_INPUT?.[0]?.nodes || [])
      .map(node => (typeof node == "string" ? node : node.sym))
      .filter(sym => !sym.startsWith("SPEC_") && win.SYM_INFO?.[sym]?.tree == "tests");
  };
  ok((await linked(QUERY)).length > 0, "The diagram has the tests tree's code citing the algorithm");
  is((await linked(`${QUERY} -tree:tests`)).length, 0, "but not with -tree:tests");
  is((await linked(`${QUERY} tree:webkit`)).length, 0, "nor with a tree: term which doesn't list it");
  ok((await linked(`${QUERY} tree:tests`)).length > 0, "but with one which does");
});
