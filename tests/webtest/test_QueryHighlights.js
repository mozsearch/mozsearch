"use strict";

// /query/'s excerpts mark their results' hits (a semantic result's token, or a
// text search's matches outside them) and the search's other matches.

add_task(async function test_QuerySemanticHits() {
  await TestUtils.loadQuery("tests", "doublePure");

  const doc = frame.contentDocument;
  const hits = [...doc.querySelectorAll(".query-result mark.query-hit")];
  ok(hits.length > 0, "The excerpts mark their hits");
  ok(hits.every(hit => hit.textContent == "doublePure"), "which are the symbols' tokens");
  ok(hits.every(hit => hit.closest("span[data-symbols]")), "inside the tokens' spans");

  const kinds = new Set(hits.map(hit => hit.closest("details.facet-group:has(> summary > h2)")
    .querySelector(":scope > summary .query-toggle").textContent));
  ok(kinds.has("Definitions") && kinds.has("Uses"), "for definitions and uses");
});

add_task(async function test_QueryTextHits() {
  await TestUtils.loadQuery("tests", "big_header");

  const doc = frame.contentDocument;
  const text = [...doc.querySelectorAll(".query-result details.facet-group")]
    .find(group => group.querySelector(":scope > summary .query-toggle")?.textContent == "Textual Occurrences");
  ok(text, "There are textual occurrences");
  const hits = [...text.querySelectorAll("mark.query-hit")];
  ok(hits.length > 0 && hits.every(hit => hit.textContent == "big_header"),
     "whose hits are the matches");
});
