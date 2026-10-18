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

function findDiagramButton(menu, label) {
  const title = [...menu.querySelectorAll(".contextmenu-section-title.icon-brush")][0];
  ok(title, "Diagram section exists");
  return [...title.closest("li").querySelectorAll(".contextmenu-button")]
    .find(x => x.textContent === label);
}

add_task(async function test_BlockDiagramFromClass() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadPath("/tests/source/rollup/view.cpp");

  const panel = [...frame.contentDocument.querySelectorAll(`span[data-symbols="T_rollup::Panel"]`)][0];
  ok(panel, "The class's token is there");
  TestUtils.click(panel);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the class");

  const block = findDiagramButton(menu, "Block");
  ok(block, "The class has a block diagram button");
  ok(findDiagramButton(menu, "Data"), "The class has a data block diagram button");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(block);
  await loadPromise;
  is(frame.contentDocument.querySelector("#query").value, "block-diagram:'T_rollup::Panel'",
     "The block diagram's query is the class's");
});

add_task(async function test_BlockDiagramEdgeCalls() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:'Core/Rollup View'");

  const edge = [...frame.contentDocument.querySelectorAll("svg [data-symbols]")]
    .find(e => e.getAttribute("data-symbols") === "T_rollup::Panel->T_rollup::Store");
  ok(edge, "There's an edge from Panel to Store");
  TestUtils.click(edge);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the edge");

  const link = [...menu.querySelectorAll(".contextmenu-link")]
    .find(l => l.textContent === "Calls between rollup::Panel and rollup::Store");
  ok(link, "The edge has the calls between its classes");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;
  is(frame.contentDocument.querySelector("#query").value,
     "calls-between-source:'rollup::Panel' calls-between-target:'rollup::Store' depth:6",
     "The query is the calls between the classes");
});

add_task(async function test_BlockDiagramSubsystemNode() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:*");

  const node = [...frame.contentDocument.querySelectorAll("svg [data-symbols]")]
    .find(e => e.getAttribute("data-symbols") === "SUBSYS_Core/Rollup@20View");
  ok(node, "The subsystem's node is there");
  TestUtils.click(node);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the subsystem");

  ok(!findDiagramButton(menu, "Calls to"), "A subsystem has no calls diagrams");
  const block = findDiagramButton(menu, "Block");
  ok(block, "The subsystem has a block diagram button");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(block);
  await loadPromise;
  is(frame.contentDocument.querySelector("#query").value, "block-diagram:'Core/Rollup View'",
     "The block diagram's query is the subsystem's");
});

add_task(async function test_BlockDiagramEdgeData() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:'Core/Rollup View'");

  const edge = [...frame.contentDocument.querySelectorAll("svg [data-symbols]")]
    .find(e => e.getAttribute("data-symbols") === "T_rollup::Panel->T_rollup::Store");
  ok(edge, "There's an edge from Panel to Store");
  TestUtils.click(edge);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the edge");

  const link = [...menu.querySelectorAll(".contextmenu-link")]
    .find(l => l.textContent === "Data between rollup::Panel and rollup::Store");
  ok(link, "The edge has the data between its classes");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;
  is(frame.contentDocument.querySelector("#query").value,
     "block-diagram:'T_rollup::Panel' block-level:field block-with:'T_rollup::Store' hier:pretty",
     "The query is the first class's fields, with the second's");
  // (Panel::Edit writes Store's mValue, which reaches Panel's mShown.)
  // (Fields' rows' texts are indented.)
  const texts = svgTexts().map(t => t.trim());
  ok(texts.includes("mValue") && texts.includes("mShown"),
     `Both classes' fields are shown: ${texts.join(", ")}`);
  ok(!texts.includes("Controller"), "but not the other classes'");
});

add_task(async function test_BlockDiagramSubsystemEdge() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "block-diagram:*");

  const edge = [...frame.contentDocument.querySelectorAll("svg [data-symbols]")]
    .find(e => e.getAttribute("data-symbols") === "SUBSYS_Core/Rollup@20View->SUBSYS_Core/Rollup@20Model");
  ok(edge, "There's an edge from the view's subsystem to the model's");
  TestUtils.click(edge);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the edge");

  const links = [...menu.querySelectorAll(".contextmenu-link")].map(l => l.textContent);
  ok(links.includes("Data between Core/Rollup View and Core/Rollup Model"),
     `The edge has the data between the subsystems' classes: ${links.join(", ")}`);
  const link = [...menu.querySelectorAll(".contextmenu-link")]
    .find(l => l.textContent === "Classes between Core/Rollup View and Core/Rollup Model");
  ok(link, "and the calls between their classes");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;
  is(frame.contentDocument.querySelector("#query").value,
     "block-diagram:'SUBSYS_Core/Rollup@20View' block-level:class block-with:'SUBSYS_Core/Rollup@20Model'",
     "The query is the first subsystem's classes, with the second's");
  const texts = svgTexts();
  ok(texts.includes("Panel") && texts.includes("Store"), `Their classes are shown: ${texts.join(", ")}`);
  ok(!texts.includes("Controller"), "but not the classes without calls between them");
});
