"use strict";

// A Wattsi spec's state machine, as the HTML parser's tokenizer's: each state
// is an algorithm right after a heading defining its term (the test tree's
// Wattsi spec's "Data state" and "Tag open state", which switch to each
// other; see tests/tests/files/specs/source), so the states are callable, and
// their references to the states they switch to are calls (see
// tools/src/spec_analyze/wattsi.rs's `defining_headings`).

const BASE = "/tests/rendered/specs/source";

add_task(async function test_SpecStateHeadings() {
  await TestUtils.loadPath(`${BASE}/gadgets.html#data-state`);
  const doc = frame.contentDocument;
  const dfn = doc.querySelector("#data-state dfn");
  is(dfn?.dataset.symbols, "SPEC_example_data@2Dstate",
     "A state's heading's term is its symbol, for the context menu");
  is(doc.querySelector("#data-state .sp-algorithm-link")?.getAttribute("href"),
     "/tests/algorithm/SPEC_example_data@2Dstate",
     "and links to its algorithm's view");

  await TestUtils.loadPath("/tests/algorithm/SPEC_example_data@2Dstate");
  const steps = frame.contentDocument.querySelector("#rendered-doc")?.textContent || "";
  ok(steps.includes("Switch to the tag open state"), "whose view has the state's algorithm");
  ok(!steps.includes("Reconsume"), "and only it");
});

add_task(async function test_SpecStateDiagrams() {
  await TestUtils.loadPath(
    "/tests/query/default?q=calls-from-sym:SPEC_example_data@2Dstate%20depth:2");
  let content = frame.contentDocument.body;
  await waitForCondition(() => content.querySelector("svg"), "The calls diagram from a state renders");
  const fromText = content.querySelector("svg").textContent;
  ok(fromText.includes("Tag open state"), "with the state it switches to");
  ok(!fromText.includes("widget"), "but not the terms it uses, which aren't calls");

  await TestUtils.loadPath(
    "/tests/query/default?q=calls-to-sym:SPEC_example_tag@2Dopen@2Dstate%20depth:2");
  content = frame.contentDocument.body;
  await waitForCondition(() => content.querySelector("svg"), "The calls diagram to a state renders");
  ok(content.querySelector("svg").textContent.includes("Data state"),
     "with the state switching to it");
});
