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

add_task(async function test_DiagramIgnore() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:10");

  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F1Ev"]`),
     "F1 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F2Ev"]`),
     "F2 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F3Ev"]`),
     "F3 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F4Ev"]`),
     "F4 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F5Ev"]`),
     "F5 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F6Ev"]`),
     "F6 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F7Ev"]`),
     "F7 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F8Ev"]`),
     "F8 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F9Ev"]`),
     "F9 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore3F10Ev"]`),
     "F10 is shown");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:10 ignore-nodes:'diagram_ignore::F6'");

  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F1Ev"]`),
     "F1 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F2Ev"]`),
     "F2 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F3Ev"]`),
     "F3 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F4Ev"]`),
     "F4 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F5Ev"]`),
     "F5 is shown");
  ok(!frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F6Ev"]`),
     "F6 is not shown");
  ok(!frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F7Ev"]`),
     "F7 is not shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F8Ev"]`),
     "F8 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F9Ev"]`),
     "F9 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore3F10Ev"]`),
     "F10 is shown");
});

add_task(async function test_DiagramIgnore_control() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:10 ignore-nodes:'diagram_ignore::F6'");

  const { panel, apply } = await openPanel();

  const ignoreNodes = panel.querySelector("#diagram-option-ignore-nodes");
  is(ignoreNodes.value, "diagram_ignore::F6", "The specified ignore nodes is shown");

  TestUtils.setText(ignoreNodes, "diagram_ignore::F2");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(apply);
  await loadPromise;

  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F1Ev"]`),
     "F1 is shown");
  ok(!frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F2Ev"]`),
     "F2 is not shown");
  ok(!frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F3Ev"]`),
     "F3 is not shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F4Ev"]`),
     "F4 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F5Ev"]`),
     "F5 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F6Ev"]`),
     "F6 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F7Ev"]`),
     "F7 is shown");
  ok(!frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F8Ev"]`),
     "F8 is not shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore2F9Ev"]`),
     "F9 is shown");
  ok(frame.contentDocument.querySelector(`g[data-symbols="_ZN14diagram_ignore3F10Ev"]`),
     "F10 is shown");
});
