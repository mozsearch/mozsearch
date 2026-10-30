"use strict";

// The facets of a client-side diagram's nodes, in the navigation panel (see
// `graph_facets` in facets.rs): their trees, if they're in several, and
// subsystems, and their classes, which leave nodes out of the diagram (see
// graph.js's `setHidden`) as they do a list's items.

const blockTexts = () => [...frame.contentDocument.querySelectorAll(".interactive-graph-block-text")]
  .map(text => text.textContent).sort().join(", ");
const facetValue = (facet, name) => [...frame.contentDocument.querySelectorAll(`.facet[data-facet="${facet}"] .facet-value`)]
  .find(button => button.firstChild.textContent.trim() == name);
const facetStatus = () => frame.contentDocument.querySelector(".facet-status").textContent;

// The searchfox tree's diagram of the example spec's poke has the tests
// tree's code citing it (its linked tree for specs' symbols, whose
// definitions it has too), and the searchfox tree's own test citing it.
add_task(async function test_DiagramFacets_trees() {
  await TestUtils.resetFeatureGate("diagramming");
  const QUERY = "calls-to-sym:'SPEC_example_concept@2Dwidget@2Dpoke' depth:2 graph-format:mozsearch-interactive";
  await TestUtils.loadQuery("searchfox", QUERY);
  const doc = frame.contentDocument;
  const facet = doc.querySelector('.facet[data-facet="subsystem"]');
  ok(facet?.querySelector(".facet-label").textContent.startsWith("Tree and subsystem"),
     "The panel has the nodes' trees' and subsystems' facet");
  const tests = facetValue("subsystem", "tests");
  ok(facetValue("subsystem", "searchfox") && tests, "with the diagram's tree and the linked one");
  const all = blockTexts();
  ok(all.includes("Poke") && all.includes("test_example_spec_citation"),
     "The diagram has both trees' nodes");

  TestUtils.click(tests, { bubbles: true, shiftKey: true });
  await waitForCondition(() => !blockTexts().includes("Poke"), "Leaving out the linked tree hides its nodes");
  ok(blockTexts().includes("test_example_spec_citation"), "but not the diagram's tree's");
  ok(/^Showing \d+ of \d+ nodes\.$/.test(facetStatus()), "and the status counts the nodes");
  ok(new URLSearchParams(frame.contentWindow.location.search).getAll("-facet-subsystem").includes("tests/"),
     "The URL leaves it out too");

  TestUtils.click(tests);
  await waitForCondition(() => !blockTexts().includes("test_example_spec_citation"),
                         "Clicking it then shows only its nodes");
  ok(blockTexts().includes("Poke"), "which are back");

  // The links beside its label query the traversal with the `tree:` terms.
  const links = [...facet.querySelectorAll("a.facet-action")];
  is(links.map(link => link.textContent).join(", "), "only searchfox, without tests",
     "Its label has links traversing only the diagram's tree, or without the linked one");
  const only = new URL(links[0].href);
  is(only.searchParams.get("q"), `${QUERY} tree:searchfox`, "The first's query has a tree: term of the diagram's tree");
  ok(new URL(links[1].href).searchParams.get("q").endsWith(" -tree:tests"), "and the second's a -tree: term");
  await TestUtils.loadPath(only.pathname + only.search);
  ok(!blockTexts().includes("Poke"), "whose diagram doesn't have the linked tree's code");
  is([...frame.contentDocument.querySelectorAll("a.facet-action")].map(link => link.textContent).join(", "),
     "all trees", "and whose link queries it without the term");
});

// rollup::Engine::Run (in Core/Rollup Engine) calls rollup::Storage's Put and
// Get (in Core/Rollup Storage).
add_task(async function test_DiagramFacets_classes() {
  await TestUtils.loadQuery("tests", "calls-from:'rollup::Engine::Run' depth:2 graph-format:mozsearch-interactive");
  is(blockTexts(), "Get, Put, Run", "The diagram has Run, Put and Get");
  ok(!frame.contentDocument.querySelector("a.facet-action"), "Without linked trees, there are no tree links");
  ok(facetValue("subsystem", "Core"), "Its subsystems' facet is hierarchical");

  TestUtils.click(facetValue("class", "Storage"), { bubbles: true, shiftKey: true });
  await waitForCondition(() => blockTexts() == "Run", "Leaving out the Storage class hides its methods");
  is(facetStatus(), "Showing 1 of 3 nodes.", "of the three nodes");

  // (And the nodes facets leave out stay out of the clustered layout.)
  await TestUtils.loadQuery("tests", "calls-from:'rollup::Engine::Run' depth:2 graph-format:mozsearch-interactive",
                            { clusters: "1", "-facet-class": "rollup::Storage" });
  is(blockTexts(), "Run", "Its URL leaves them out of the clustered diagram too");
  TestUtils.click(frame.contentDocument.querySelector(".facet-clear"));
  await waitForCondition(() => blockTexts() == "Get, Put, Run", "and showing all brings them back");
  ok(frame.contentDocument.querySelector(".interactive-graph-cluster"), "in their clusters");
});

// A diagram whose nodes' facets would leave nothing out (one tree, one
// subsystem, no classes) has no box of facets.
add_task(async function test_DiagramFacets_none() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");
  ok(frame.contentDocument.querySelector(".interactive-graph-block"), "The diagram is there");
  ok(!frame.contentDocument.querySelector(".facet-box"), "without facets");
});
