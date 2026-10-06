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
  ok(blots.every(blot => blot.querySelectorAll("i").length > 0 &&
                         [...blot.querySelectorAll("i")].every(i => /^b\d r[1-5]$/.test(i.className))),
     "with squares for the bins of age with changes");
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

add_task(async function test_QueryTextRecency() {
  // Textual occurrences are as recent as their contexts, and lines at their
  // files' top levels (ex: `use`s) or in namespaces are as recent as those
  // scopes in their files, as file name matches are as recent as their files.
  await TestUtils.loadQuery("searchfox", "recency_html");

  const doc = frame.contentDocument;
  const group = name => [...doc.querySelectorAll(".query-result details.facet-group")]
    .find(details => details.querySelector(":scope > summary .query-toggle")?.textContent == name);

  const files = group("Files");
  ok(files?.querySelector(".query-file-names li .query-recency[title]"),
     "File name matches have their files' blots");

  // (In files with and without analysis, ex: docs and CSS, whose lines have
  // their files' blots.)
  const text = group("Textual Occurrences");
  ok(text, "There are textual occurrences");
  const textLines = [...text.querySelectorAll(".file")];
  ok(textLines.length > 0 && textLines.every(line => line.querySelector(".query-recency[title]")),
     "which have blots");

  // (Including a Rust module's, whose scope is the module's own name.)
  const uses = [...doc.querySelectorAll(".query-result .file")]
    .filter(line => /^\s*use\b/.test(line.querySelector("code")?.textContent || ""));
  ok(uses.length > 0, "There are `use` lines");
  ok(uses.every(line => line.querySelector(".query-recency[title]")),
     "which have their scopes' blots");
});
