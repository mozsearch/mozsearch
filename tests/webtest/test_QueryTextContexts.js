"use strict";

// /query/'s textual occurrences say what they're in ("// found in"), like
// crossref's results: the innermost nesting container (ex: function) around
// them, from the rendered file (see `chunked_gzip`).

add_task(async function test_QueryTextContexts() {
  await TestUtils.loadQuery("tests", "shoelace");

  const doc = frame.contentDocument;
  const text = [...doc.querySelectorAll(".query-result details.facet-group")]
    .find(group => group.querySelector(":scope > summary > h2 .query-toggle")?.textContent == "Textual Occurrences");
  ok(text, "There are textual occurrences");

  const context = line => text.querySelector(`#line-${line} .result-context`)?.textContent;
  is(context(25), "// found in ShoelaceRunnable::Run",
     "A string in a method is in the method");
  is(context(37), "// found in ShoeRunnable::Run",
     "as is a local's use, in a later chunk of the file");
  ok(text.querySelector("#line-37 .result-context a")?.href.includes("symbol:"),
     "with a link to the method's symbol");
});
