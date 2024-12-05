"use strict";

add_task(async function test_MacroExpansionsContextMenu() {
  // Enabled by default on the test config.
  await TestUtils.resetFeatureGate("expansions");
  await TestUtils.loadPath("/tests/source/macro.cpp");

  const perTargetFunctionExpansionPoint = frame.contentDocument.querySelector("span[data-expansions*=per_target_function]");
  TestUtils.click(perTargetFunctionExpansionPoint);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for macro click");

  const expansions = JSON.parse(perTargetFunctionExpansionPoint.dataset.expansions);
  is(Object.keys(expansions).length, 3, "3 expansions are available");

  const expansionRows = menu.querySelectorAll(".contextmenu-expansion-preview");
  is(expansionRows.length, 3, "3 expansion rows are visible");

  const expectedPlatform = "win64";
  const expansionRow = expansionRows[0];
  // needs to be cancelable because context menu actions are <a href="#> and use preventDefault on click
  TestUtils.click(expansionRow, { bubbles: true, cancelable: true });

  const blamePopup = frame.contentWindow.BlamePopup;
  const blameStripHoverHandler = frame.contentWindow.BlameStripHoverHandler;
  await waitForShown(blamePopup.popup, "BlamePopup is shown");

  ok(blameStripHoverHandler.keepVisible, "BlamePopup won't be dismissed on mouseleave");
  is(blamePopup.popupOwner, perTargetFunctionExpansionPoint, "BlamePopup is related to the macro use");
  ok(blamePopup.popup.innerHTML.includes(expectedPlatform), "BlamePopup shows the right platform");

  const functionDefinition = blamePopup.popup.querySelector("span.syn_def[data-symbols=_Z19per_target_functionv]");
  TestUtils.click(functionDefinition);

  const inCodeMenu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(inCodeMenu, "Context menu is shown for function definition in macro expansion");

  const macroSpanDefinition = blamePopup.popup.querySelector(`span[data-symbols*="M_"]`);
  TestUtils.click(macroSpanDefinition);

  const inTitleMenu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(inTitleMenu, "Context menu is shown for the macro name in the title");
});

add_task(async function test_MacroExpansionsContextMenu_gate() {
  registerCleanupFunction(async () => {
    await TestUtils.resetFeatureGate("expansions");
  });

  const tests = [
    { value: "release", shown: false },
    { value: "beta", shown: false },
    { value: "alpha", shown: true },
  ];
  for (const { value, shown } of tests) {
    await TestUtils.setFeatureGate("expansions", value);
    await TestUtils.loadPath("/tests/source/macro.cpp");

    const perTargetFunctionExpansionPoint = frame.contentDocument.querySelector("span[data-expansions*=per_target_function]");
    TestUtils.click(perTargetFunctionExpansionPoint);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for symbol click");

    const expansions = menu.querySelector(".contextmenu-expansion-preview");
    if (shown) {
      ok(!!expansions, `Expansion menu items exist on ${value}`);
    } else {
      ok(!expansions, `Expansion menu items do not exist on ${value}`);
    }
  }
});
