add_task(async function test_DiagramIcon_hint_depth() {
  await TestUtils.loadQuery("tests", "calls-between:'diagram_ignore::F10' calls-between:'diagram_ignore::F2' depth:3");

  const hint = frame.contentDocument.querySelector(".diagram-no-path-hint");
  ok(!!hint, "Hint is shown");

  const buttons = hint.querySelectorAll("button");
  is(buttons.length, 1, "button is shown in the hint");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(buttons[0]);
  await loadPromise;

  const query = frame.contentDocument.querySelector(`#query`);
  is(query.value, "calls-between:'diagram_ignore::F10' calls-between:'diagram_ignore::F2' depth:4",
     "depth is increased");

  {
    const F9 = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F9Ev"]`);
    ok(F9, "F9 is shown");
  }
});

add_task(async function test_DiagramIcon_hint_flip() {
  await TestUtils.loadQuery("tests", "calls-between-target:'diagram_ignore::F10' calls-between-source:'diagram_ignore::F2' depth:4");

  const hint = frame.contentDocument.querySelector(".diagram-no-path-hint");
  ok(!!hint, "Hint is shown");

  const buttons = hint.querySelectorAll("button");
  is(buttons.length, 3, "button is shown in the hint");

  is(buttons[0].textContent, "Flip the direction",
     "A button to flip the direction is shown");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(buttons[0]);
  await loadPromise;

  const query = frame.contentDocument.querySelector(`#query`);
  is(query.value, "calls-between-source:'diagram_ignore::F10' calls-between-target:'diagram_ignore::F2' depth:4",
     "direction is flipped");

  {
    const F9 = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F9Ev"]`);
    ok(F9, "F9 is shown");
  }
});

add_task(async function test_DiagramIcon_hint_undirected() {
  await TestUtils.loadQuery("tests", "calls-between-target:'diagram_ignore::F10' calls-between-source:'diagram_ignore::F2' depth:4");

  const hint = frame.contentDocument.querySelector(".diagram-no-path-hint");
  ok(!!hint, "Hint is shown");

  const buttons = hint.querySelectorAll("button");
  is(buttons.length, 3, "button is shown in the hint");

  is(buttons[1].textContent, "Use undirected diagram",
     "A button to convert to a undirected diagram is shown");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(buttons[1]);
  await loadPromise;

  const query = frame.contentDocument.querySelector(`#query`);
  is(query.value, "calls-between:'diagram_ignore::F10' calls-between:'diagram_ignore::F2' depth:4",
     "direction is flipped");

  {
    const F9 = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F9Ev"]`);
    ok(F9, "F9 is shown");
  }
});
