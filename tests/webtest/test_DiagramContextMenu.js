function findDiagramListItem(menu) {
  const brushRows = menu.querySelectorAll(".icon-brush");
  const row = [...brushRows].find(x => x.classList.contains("contextmenu-section-title"));
  ok(row, "Diagram section exists");
  return row.closest("li");
}

add_task(async function test_DiagramContextMenu_CallsTo() {
  await TestUtils.resetFeatureGate("diagramming");
  frame.contentWindow.localStorage.removeItem("diagram-pinned");

  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
  TestUtils.click(f2);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const listItem = findDiagramListItem(menu);
  const buttons = listItem.querySelectorAll(".contextmenu-button");
  const button = [...buttons].find(x => x.textContent === "Calls to");
  ok(button, "Button exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(button);
  await loadPromise;

  ok(frame.contentDocument.location.href.includes("/query/"),
     "Navigated to query page");

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "calls-to:'diagram_ignore::F2' depth:4",
     "Query is set");
});

add_task(async function test_DiagramContextMenu_CallsTo() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
  TestUtils.click(f2);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const listItem = findDiagramListItem(menu);
  const buttons = listItem.querySelectorAll(".contextmenu-button");
  const button = [...buttons].find(x => x.textContent === "Calls from");
  ok(button, "Button exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(button);
  await loadPromise;

  ok(frame.contentDocument.location.href.includes("/query/"),
     "Navigated to query page");

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "calls-from:'diagram_ignore::F2' depth:4",
     "Query is set");
});

add_task(async function test_DiagramContextMenu_Class_state() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_class.cpp");

  {
    const c = frame.contentDocument.querySelector(`span[data-symbols="T_diagram_class::WithoutFields"]`);
    TestUtils.click(c);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);
    const buttons = listItem.querySelectorAll(".contextmenu-button");

    const classButton = [...buttons].find(x => x.textContent === "Class");
    ok(classButton, "Class button exists");
    ok(classButton.classList.contains("disabled"),
       "Class button is disabled");

    const inheritanceButton = [...buttons].find(x => x.textContent === "Inheritance");
    ok(inheritanceButton, "Inheritance button exists");
    ok(inheritanceButton.classList.contains("disabled"),
       "Inheritance button is disabled");
  }

  {
    const c = frame.contentDocument.querySelector(`span[data-symbols="T_diagram_class::WithFields"]`);
    TestUtils.click(c);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);
    const buttons = listItem.querySelectorAll(".contextmenu-button");

    const classButton = [...buttons].find(x => x.textContent === "Class");
    ok(classButton, "Class button exists");
    ok(!classButton.classList.contains("disabled"),
       "Class button is enabled");

    const inheritanceButton = [...buttons].find(x => x.textContent === "Inheritance");
    ok(inheritanceButton, "Inheritance button exists");
    ok(inheritanceButton.classList.contains("disabled"),
       "Inheritance button is disabled");
  }

  {
    const c = frame.contentDocument.querySelector(`span[data-symbols="T_diagram_class::Superclass"]`);
    TestUtils.click(c);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);
    const buttons = listItem.querySelectorAll(".contextmenu-button");

    const classButton = [...buttons].find(x => x.textContent === "Class");
    ok(classButton, "Class button exists");
    ok(classButton.classList.contains("disabled"),
       "Class button is disabled");

    const inheritanceButton = [...buttons].find(x => x.textContent === "Inheritance");
    ok(inheritanceButton, "Inheritance button exists");
    ok(!inheritanceButton.classList.contains("disabled"),
       "Inheritance button is enabled");
  }

  {
    const c = frame.contentDocument.querySelector(`span[data-symbols="T_diagram_class::Subclass"]`);
    TestUtils.click(c);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);
    const buttons = listItem.querySelectorAll(".contextmenu-button");

    const classButton = [...buttons].find(x => x.textContent === "Class");
    ok(classButton, "Class button exists");
    ok(!classButton.classList.contains("disabled"),
       "Class button is enabled");

    const inheritanceButton = [...buttons].find(x => x.textContent === "Inheritance");
    ok(inheritanceButton, "Inheritance button exists");
    ok(!inheritanceButton.classList.contains("disabled"),
       "Inheritance button is enabled");

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(classButton);
    await loadPromise;

    ok(frame.contentDocument.location.href.includes("/query/"),
       "Navigated to query page");

    const query = frame.contentDocument.querySelector("#query");
    is(query.value, "class-diagram:'diagram_class::Subclass' depth:4",
       "Query is set");
  }
});

add_task(async function test_DiagramContextMenu_Inheritance() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_class.cpp");

  const c = frame.contentDocument.querySelector(`span[data-symbols="T_diagram_class::Subclass"]`);
  TestUtils.click(c);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const listItem = findDiagramListItem(menu);
  const buttons = listItem.querySelectorAll(".contextmenu-button");

  const classButton = [...buttons].find(x => x.textContent === "Class");
  ok(classButton, "Class button exists");
  ok(!classButton.classList.contains("disabled"),
     "Class button is enabled");

  const inheritanceButton = [...buttons].find(x => x.textContent === "Inheritance");
  ok(inheritanceButton, "Inheritance button exists");
  ok(!inheritanceButton.classList.contains("disabled"),
     "Inheritance button is enabled");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(inheritanceButton);
  await loadPromise;

  ok(frame.contentDocument.location.href.includes("/query/"),
     "Navigated to query page");

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "inheritance-diagram:'diagram_class::Subclass' depth:4",
     "Query is set");
});

add_task(async function test_DiagramContextMenu_pin() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  {
    const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
    TestUtils.click(f1);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const pin = [...buttons].find(x => x.classList.contains("icon-pin"));
    ok(pin, "Pin button exists");

    TestUtils.click(pin);

    is(window.getComputedStyle(menu).display, "none",
       "menu is closed");
  }

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(subsection, "Pinned section is shown");
    is(subsection.textContent, "with pinned = diagram_ignore::F1", "Pinned item is shown");

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const unpin = [...buttons].find(x => x.classList.contains("icon-trash-empty"));
    ok(unpin, "Unpin button exists");

    TestUtils.click(unpin);

    is(window.getComputedStyle(menu).display, "none",
       "menu is closed");
  }

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");
  }
});

add_task(async function test_DiagramContextMenu_CallsBetweenTo() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const pin = [...buttons].find(x => x.classList.contains("icon-pin"));
    ok(pin, "Pin button exists");

    TestUtils.click(pin);

    is(window.getComputedStyle(menu).display, "none",
       "menu is closed");
  }

  {
    const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
    TestUtils.click(f1);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const button = [...buttons].find(x => x.textContent === "Calls from pinned to");
    ok(button, "Button exists");

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(button);
    await loadPromise;

    ok(frame.contentDocument.location.href.includes("/query/"),
       "Navigated to query page");

    const query = frame.contentDocument.querySelector("#query");
    is(query.value, "calls-between-source:'diagram_ignore::F2' calls-between-target:'diagram_ignore::F1' depth:8",
       "Query is set");
  }

  {
    await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(subsection, "Pinned section is still shown");
    is(subsection.textContent, "with pinned = diagram_ignore::F2", "Pinned item is shown");
  }

  frame.contentWindow.localStorage.removeItem("diagram-pinned");
});

add_task(async function test_DiagramContextMenu_CallsBetweenFrom() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  {
    const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
    TestUtils.click(f1);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const pin = [...buttons].find(x => x.classList.contains("icon-pin"));
    ok(pin, "Pin button exists");

    TestUtils.click(pin);

    is(window.getComputedStyle(menu).display, "none",
       "menu is closed");
  }

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const button = [...buttons].find(x => x.textContent === "Calls to pinned from");
    ok(button, "Button exists");

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(button);
    await loadPromise;

    ok(frame.contentDocument.location.href.includes("/query/"),
       "Navigated to query page");

    const query = frame.contentDocument.querySelector("#query");
    is(query.value, "calls-between-source:'diagram_ignore::F2' calls-between-target:'diagram_ignore::F1' depth:8",
       "Query is set");
  }

  frame.contentWindow.localStorage.removeItem("diagram-pinned");
});

add_task(async function test_DiagramContextMenu_CallsBetweenUndirected() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  {
    const f1 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
    TestUtils.click(f1);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const pin = [...buttons].find(x => x.classList.contains("icon-pin"));
    ok(pin, "Pin button exists");

    TestUtils.click(pin);

    is(window.getComputedStyle(menu).display, "none",
       "menu is closed");
  }

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const buttons = listItem.querySelectorAll(".contextmenu-button");
    const button = [...buttons].find(x => x.textContent === "Calls between");
    ok(button, "Button exists");

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(button);
    await loadPromise;

    ok(frame.contentDocument.location.href.includes("/query/"),
       "Navigated to query page");

    const query = frame.contentDocument.querySelector("#query");
    is(query.value, "calls-between:'diagram_ignore::F1' calls-between:'diagram_ignore::F2' depth:8",
       "Query is set");
  }

  frame.contentWindow.localStorage.removeItem("diagram-pinned");
});

add_task(async function test_DiagramContextMenu_sync() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_ignore.cpp");

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");
  }

  frame.contentWindow.localStorage.setItem("diagram-pinned", "Func1");

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(subsection, "Pinned section is shown");
    is(subsection.textContent, "with pinned = Func1", "Pinned item is shown");
  }

  frame.contentWindow.localStorage.removeItem("diagram-pinned");

  {
    const f2 = frame.contentDocument.querySelector(`span[data-symbols="_ZN14diagram_ignore2F2Ev"]`);
    TestUtils.click(f2);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const listItem = findDiagramListItem(menu);

    const subsection = listItem.querySelector(".contextmenu-subsection-title");
    ok(!subsection, "Pinned section is not shown");
  }
});

add_task(async function test_DiagramContextMenu_merge() {
  await TestUtils.loadPath("/tests/source/cpp/diagram_merge.cpp");

  {
    const foo = frame.contentDocument.querySelector(`span[data-symbols*="_ZN13diagram_merge3foo"]`);
    TestUtils.click(foo);

    const menu = frame.contentDocument.querySelector("#context-menu");
    const brushRows = menu.querySelectorAll(".icon-brush");
    is(brushRows.length, 1, "Only one diagram menu item is shown");
  }
});
