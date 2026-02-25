add_task(async function test_DiagramIcon_Depth() {
  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:2");

  {
    const F6 = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F6Ev"]`);
    ok(!F6, "F6 is not shown");
  }

  const icons = frame.contentDocument.querySelectorAll(".diagram-overload-depth");
  is(icons.length, 3, "two icons are shown");

  TestUtils.click(icons[0]);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for icon click");

  const button = menu.querySelector(".contextmenu-button");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(button);
  await loadPromise;

  const query = frame.contentDocument.querySelector(`#query`);
  is(query.value, "calls-to:'diagram_ignore::F1' depth:3",
     "depth is increased");

  {
    const F6 = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F6Ev"]`);
    ok(F6, "F6 is shown");
  }
});

add_task(async function test_DiagramIcon_NonDepth() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_lines_local::target' depth:4");

  {
    const caller19 = frame.contentDocument.querySelector(`[data-symbols="_ZN7diagram16uses_lines_local8caller19Ev"]`);
    ok(!caller19, "caller19 is not shown");
  }

  const icon = frame.contentDocument.querySelector(".diagram-overload-other");
  ok(icon,"icon is present");

  TestUtils.click(icon);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for icon click");

  const button = menu.querySelector(".contextmenu-button");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(button);
  await loadPromise;

  {
    const caller19 = frame.contentDocument.querySelector(`[data-symbols="_ZN7diagram16uses_lines_local8caller19Ev"]`);
    ok(caller19, "caller19 is shown");
  }
});
