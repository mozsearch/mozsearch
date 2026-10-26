"use strict";

// /query/'s results say what contains each line ("// found in CONTEXT") at the
// end of the line, like /search/, with the context's symbol, for its context
// menu, and only when there is a context.
add_task(async function test_QueryFoundIn() {
  await TestUtils.loadQuery("tests", "doublePure");

  const doc = frame.contentDocument;
  const contexts = [...doc.querySelectorAll(".result-context")];
  ok(contexts.length > 0, "Lines say what they're found in");
  ok(contexts.every(context => context.closest("code.source-line")), "on the lines themselves");
  ok(contexts.every(context => context.querySelector("code").textContent.trim() != ""),
     "and only when there's a context");
  const symbol = contexts
    .map(context => context.querySelector("[data-symbols]"))
    .find(span => span && span.textContent == "DoubleBase");
  is(symbol?.dataset.symbols, "T_DoubleBase", "The context is its symbol");
  ok(frame.contentWindow.SYM_INFO.T_DoubleBase, "whose info the page has");
});
