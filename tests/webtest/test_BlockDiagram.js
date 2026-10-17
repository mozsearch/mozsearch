"use strict";

// Block diagrams (see tools/src/cmd_pipeline/cmd_block_diagram.rs), from
// crossref's roll-ups of tests/tests/files/rollup/'s calls and data.

function svgTexts() {
  return [...frame.contentDocument.querySelectorAll("svg text")].map(t => t.textContent);
}

add_task(async function test_BlockDiagramCalls() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:'Core/Rollup View'");

  const texts = svgTexts();
  ok(texts.includes("Panel") && texts.includes("Controller") && texts.includes("Store"),
     `The subsystem's classes and their neighbor are shown: ${texts.join(", ")}`);
  ok(texts.includes("Rollup Model") && texts.includes("Rollup View"),
     "The classes are clustered by subsystem");
  // (Panel calls Store's Put and Get, and Controller calls Panel's Edit and
  // Refresh: each edge is labeled with its two calls.)
  is(texts.filter(t => t === "2").length, 2, "The edges are labeled with their calls");

  // (Edges' symbols are "<from>-><to>"; see `transform_svg`.)
  const edge = [...frame.contentDocument.querySelectorAll("svg [data-symbols]")]
    .find(e => e.getAttribute("data-symbols") === "T_rollup::Panel->T_rollup::Store");
  ok(edge, "There's an edge from Panel to Store");
});

add_task(async function test_BlockDiagramData() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:'Core/Rollup View' block-edges:data");

  const texts = svgTexts();
  ok(texts.includes("Controller") && texts.includes("Store") && texts.includes("Panel"),
     `The classes data flows between are shown: ${texts.join(", ")}`);
  // (Controller's mInput reaches Store's mValue, which reaches Panel's
  // mShown, and Panel::Edit writes Store's mValue.)
  const edges = [...frame.contentDocument.querySelectorAll("svg [data-symbols]")]
    .map(e => e.getAttribute("data-symbols"));
  for (const edge of ["T_rollup::Controller->T_rollup::Store",
                      "T_rollup::Store->T_rollup::Panel",
                      "T_rollup::Panel->T_rollup::Store"]) {
    ok(edges.includes(edge), `There's a data edge ${edge}`);
  }
});

add_task(async function test_BlockDiagramPanel() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:'Core/Rollup View'");

  const toggle = frame.contentDocument.querySelector("#diagram-panel-toggle");
  TestUtils.click(toggle);
  const panel = frame.contentDocument.querySelector("#diagram-panel");
  await waitForCondition(() => !panel.classList.contains("hidden"), "Panel is shown");

  const level = panel.querySelector("#diagram-option-block-level");
  ok(level, "The level is a control");
  is(level?.value, "class", "The level is classes");
  const minCount = panel.querySelector("#diagram-option-block-min-count");
  is(minCount?.value, "1", "The minimum count is the default");
});
