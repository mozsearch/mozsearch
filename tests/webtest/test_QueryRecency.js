"use strict";

// /query/'s results show how recently, and how much, their code changed (see
// `file_format::recency`), for trees with histories (like "searchfox"): a blot
// on each result's key line, and a "Last changed" facet whose values filter the
// lines.

add_task(async function test_QueryRecency() {
  await TestUtils.loadQuery("searchfox", "blot_cell");

  const doc = frame.contentDocument;
  const blots = [...doc.querySelectorAll(".query-result .query-recency[title]")];
  ok(blots.length > 0, "The results' key lines have blots");
  ok(blots.every(blot => blot.querySelectorAll("i").length == 10),
     "with a square per bin of age");
  ok(blots.every(blot => /tokens? changed/.test(blot.title)), "which say what changed");
  const rows = [...doc.querySelectorAll(".query-result div[role=row]")];
  ok(rows.every(row => row.querySelector(":scope > .query-recency")),
     "Every row has the blots' cell, to keep the code aligned");

  const facet = doc.querySelector('.facet[data-facet="recency"]');
  ok(facet, "There's a Last changed facet");
  ok(facet.querySelector(".facet-value .recency-sparkline"), "whose values have sparklines");

  // Selecting a value only shows the lines it's for.
  const button = facet.querySelector(".facet-value");
  const value = button.dataset.value;
  TestUtils.click(button);
  const lines = [...doc.querySelectorAll(".query-result .file[data-facets]")];
  ok(lines.length > 0, "The lines are facet items");
  const of = line => JSON.parse(line.dataset.facets).recency[0];
  ok(lines.every(line => line.hidden == (of(line) != value)),
     `Selecting "${button.textContent.trim()}" shows its lines and hides the others`);
  TestUtils.click(button);
  ok(lines.every(line => !line.hidden), "and unselecting it shows them all");
});
