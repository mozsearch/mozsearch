"use strict";

async function openPanel() {
  const toggle = frame.contentDocument.querySelector("#diagram-panel-toggle");
  TestUtils.click(toggle);

  const panel = frame.contentDocument.querySelector("#diagram-panel");
  await waitForCondition(() => !panel.classList.contains("hidden"),
                         "Panel is shown");

  await waitForCondition(() => panel.querySelector("button"),
                         "Apply button is shown");
  const apply = panel.querySelector("button");

  return { panel, apply };
}

add_task(async function test_DiagramControl_basic() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "calls-to:'diagram::uses_lines_local::target' depth:4",
     "The specified query is shown");

  const { panel, apply } = await openPanel();

  const depth = panel.querySelector("#diagram-option-depth");
  is(depth.value, "4", "The specified depth is shown");

  const depthRange = panel.querySelector("#diagram-option-range-depth");
  is(depth.value, "4", "Range input is in sync with text");

  const nodeLimit = panel.querySelector("#diagram-option-node-limit");
  is(nodeLimit.value, "384", "The default node limit is shown");

  const pathLimit = panel.querySelector("#diagram-option-path-limit");
  is(pathLimit.value, "0", "The default path limit is shown");

  const ignoreNodes = panel.querySelector("#diagram-option-ignore-nodes");
  is(ignoreNodes.value, "", "The default ignore nodes is shown");

  const hier = panel.querySelector("#diagram-option-hier");
  is(hier.value, "pretty", "The default hier is shown");

  const layout = panel.querySelector("#diagram-option-graph-layout");
  is(layout.value, "dot", "The default hier is shown");

  const format = panel.querySelector("#diagram-option-graph-format");
  is(format.value, "mozsearch", "The default hier is shown");

  const debug = panel.querySelector("#diagram-option-graph-debug");
  is(debug.checked, false, "The default debug is selected");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(apply);
  await loadPromise;

  const newQuery = frame.contentDocument.querySelector("#query");
  is(newQuery.value, "calls-to:'diagram::uses_lines_local::target' depth:4",
     "The query doesn't change");
});

add_task(async function test_DiagramControl_range() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  const { panel, apply } = await openPanel();

  const depth = panel.querySelector("#diagram-option-depth");
  TestUtils.setText(depth, "16");

  const depthRange = panel.querySelector("#diagram-option-range-depth");
  is(depth.value, "16", "Range input is in sync with text");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(apply);
  await loadPromise;

  const newQuery = frame.contentDocument.querySelector("#query");
  is(newQuery.value, "calls-to:'diagram::uses_lines_local::target' depth:16",
     "The query reflects the modification");
});

add_task(async function test_DiagramControl_nodeLimitWarning() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
  ok(warningBox,"warning is present");

  const { panel, apply } = await openPanel();

  const pathLimit = panel.querySelector("#diagram-option-path-limit");
  TestUtils.setText(pathLimit, "100");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(apply);
  await loadPromise;

  const newQuery = frame.contentDocument.querySelector("#query");
  is(newQuery.value, "calls-to:'diagram::uses_lines_local::target' depth:4 path-limit:100",
     "The query reflects the modification");

  const newWarningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
  ok(!newWarningBox,"warning is not present");
});

add_task(async function test_DiagramControl_menu() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  const { panel, apply } = await openPanel();

  const hier = panel.querySelector("#diagram-option-hier");
  TestUtils.selectMenu(hier, "flat");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(apply);
  await loadPromise;

  const newQuery = frame.contentDocument.querySelector("#query");
  is(newQuery.value, "calls-to:'diagram::uses_lines_local::target' depth:4 hier:flat",
     "The query reflects the modification");
});

add_task(async function test_DiagramControl_checkbox() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  const { panel, apply } = await openPanel();

  const debug = panel.querySelector("#diagram-option-graph-debug");
  TestUtils.clickCheckbox(debug);

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(apply);
  await loadPromise;

  const newQuery = frame.contentDocument.querySelector("#query");
  is(newQuery.value, "calls-to:'diagram::uses_lines_local::target' depth:4 graph-debug:true",
     "The query reflects the modification");
});
