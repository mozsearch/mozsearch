"use strict";

// /query/'s file-centric results get the /explore/ pages' facets (see
// test_Explore.js), which filter the results' files and hide the groups left
// without any.  "console" is in core code (ex: js/) and test files (ex:
// testing/).
add_task(async function test_QueryFacets() {
  await TestUtils.loadQuery("tests", "console");

  const doc = frame.contentDocument;
  const test = doc.querySelector('.facet[data-facet="kind"] .facet-value[data-value="test"]');
  ok(test, "The results have a path kind facet");
  ok(!doc.querySelector(".facet-group-by"), "but no choice of grouping, since they're grouped by kind");

  const shown = () => [...doc.querySelectorAll(".facet-items [data-facets]")].filter(item => !item.hidden);
  const all = shown().length;
  TestUtils.click(test);
  await waitForCondition(() => shown().length < all, "Selecting test files filters the results");
  ok(shown().every(item => JSON.parse(item.dataset.facets).kind.includes("test")), "to the test files");
  const pathKinds = [...doc.querySelectorAll(".facet-items > details.facet-group")];
  ok(pathKinds.some(group => group.hidden) && pathKinds.some(group => !group.hidden),
     "The groups of the other path kinds are hidden");
  ok(doc.querySelector(".facet-status").textContent.startsWith("Showing"),
     "The status says how many files are shown");
  ok(frame.contentWindow.location.search.includes("facet-kind=test"), "The selection is in the URL");
});
