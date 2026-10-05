"use strict";

// /query/'s results say what contains each line ("// found in CONTEXT") at the
// end of the line, like /search/, with a link to a search for the context's
// symbol, and only when there is a context.
add_task(async function test_QueryFoundIn() {
  await TestUtils.loadQuery("tests", "doublePure");

  const doc = frame.contentDocument;
  const contexts = [...doc.querySelectorAll(".result-context")];
  ok(contexts.length > 0, "Lines say what they're found in");
  ok(contexts.every(context => context.closest("code.source-line")), "on the lines themselves");
  ok(contexts.every(context => context.querySelector("code").textContent.trim() != ""),
     "and only when there's a context");
  const link = contexts
    .map(context => context.querySelector("a"))
    .find(a => a && a.textContent == "DoubleBase");
  ok(link && link.href.includes("q=symbol:T_DoubleBase"), "The context links to a search for its symbol");
});
