"use strict";

function findMenuLink(menu, text) {
  for (const row of menu.querySelectorAll(".contextmenu-link")) {
    if (row.textContent.includes(text)) {
      return row;
    }
  }

  return null;
}

add_task(async function test_SubMenu_Keyboard() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const line = 44;
  const sym = "#jsWebIDLConflictMethod";
  const pretty = "JSWebIDLPartialNamespace";

  const selector = `#line-${line} span[data-symbols*="${sym}"]`;
  const elem = frame.contentDocument.querySelector(selector);
  ok(!!elem, `Symbol element exists for ${sym}`);
  TestUtils.click(elem);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

  const item = findMenuLink(menu, "Possible IDL definitions");

  const tester = new KeyboardNavigationTester(menu);

  await tester.keydown("Down");
  await tester.keydown("Down");
  is(await tester.keydown("Down"), item,
     "The IDL definitions item should be focused");

  const first = await tester.keydown("Right");

  const subMenu = frame.contentDocument.querySelector(".context-submenu");
  const subItems = [...subMenu.querySelectorAll("a")];

  is(first, subItems[0],
     "The first item should be selected after opening sub menu");
  is(await tester.keydown("Down"), subItems[1]);
  is(await tester.keydown("Down"), subItems[2]);
  is(await tester.keydown("Down"), subItems[3]);
  is(await tester.keydown("Left"), item);
});

