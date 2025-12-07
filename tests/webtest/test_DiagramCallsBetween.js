add_task(async function test_DiagramCallsBetween_panel() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  const callgraphBox = frame.contentDocument.querySelector(".callgraph-box");
  is(callgraphBox.textContent, "",
     "Callgraph box should be empty");

  {
    const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
    TestUtils.click(f1);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const brushRows = menu.querySelectorAll(".icon-brush");
    is(brushRows.length, 3, "3 brush items are visible");

    is(brushRows[2].textContent, "Save as calls diagram source: diagram_ignore::F1",
       "save as source item is shown");

    TestUtils.click(brushRows[2]);

    await waitForCondition(() => callgraphBox.textContent.includes("F1"),
                           "Callgraph box should be populated");
  }

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const brushRows = menu.querySelectorAll(".icon-brush");
    is(brushRows.length, 4, "4 brush items are visible");

    is(brushRows[2].textContent, "Save as calls diagram source: diagram_ignore::F2",
       "save as source item is shown");

    TestUtils.click(brushRows[2]);

    await waitForCondition(() => callgraphBox.textContent.includes("F2"),
                           "Callgraph box should be updated");
  }

  const trash = callgraphBox.querySelector(".trash");
  ok(trash, "trash icon is shown");

  TestUtils.click(trash);

  await waitForCondition(() => callgraphBox.textContent == "",
                         "Callgraph box should be cleared");
});

add_task(async function test_DiagramCallsBetween_graph() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  const callgraphBox = frame.contentDocument.querySelector(".callgraph-box");
  is(callgraphBox.textContent, "",
     "Callgraph box should be empty");

  {
    const f10 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore3F10Ev"]`);
    TestUtils.click(f10);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const brushRows = menu.querySelectorAll(".icon-brush");
    is(brushRows.length, 3, "3 brush items are visible");

    is(brushRows[2].textContent, "Save as calls diagram source: diagram_ignore::F10",
       "save as source item is shown");

    TestUtils.click(brushRows[2]);

    await waitForCondition(() => callgraphBox.textContent.includes("F10"),
                           "Callgraph box should be populated");
  }

  {
    const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
    TestUtils.click(f1);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const brushRows = menu.querySelectorAll(".icon-brush");
    is(brushRows.length, 4, "4 brush items are visible");

    is(brushRows[3].textContent, "Use as calls diagram target: diagram_ignore::F1",
       "use as target item is shown");

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(brushRows[3]);
    await loadPromise;
  }

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "calls-between-source:diagram_ignore::F10 calls-between-target:diagram_ignore::F1 depth:8",
     "calls-between graph should be shown");

  const newCallgraphBox = frame.contentDocument.querySelector(".callgraph-box");
  is(newCallgraphBox.textContent, "",
     "Callgraph box should be empty after generating a graph");
});

add_task(async function test_DiagramCallsBetween_sync() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  const callgraphBox = frame.contentDocument.querySelector(".callgraph-box");
  is(callgraphBox.textContent, "",
     "Callgraph box should be empty");

  // Emulate a situation where the source is set by other tab.
  localStorage.setItem("callgraph-source", "diagram_ignore::F3");

  const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
  TestUtils.click(f1);

  // Opening a context menu should sync the panel.

  await waitForCondition(() => callgraphBox.textContent.includes("F3"),
                         "Callgraph box should be synced");

  localStorage.setItem("callgraph-source", "diagram_ignore::F3");

  // Emulate a situation where the source is updated by other tab.
  localStorage.setItem("callgraph-source", "diagram_ignore::F5");

  TestUtils.click(f1);

  await waitForCondition(() => callgraphBox.textContent.includes("F5"),
                         "Callgraph box should be synced");

  // Emulate a situation where the source is clearedd by other tab.
  localStorage.removeItem("callgraph-source");

  TestUtils.click(f1);

  await waitForCondition(() => callgraphBox.textContent == "",
                         "Callgraph box should be synced");
});
