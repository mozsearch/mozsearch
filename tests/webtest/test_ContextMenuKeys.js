"use strict";

add_task(async function test_TreeSwitcherKeyboardNavigation() {
  await TestUtils.setFeatureGate("diagramming", "release");

  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  const word = frame.contentDocument.querySelector(`#line-14 span[data-symbols*="T_webtest::ClassWithConsumer"]`);

  const menu = frame.contentDocument.querySelector("#context-menu");

  {
    const shownPromise = waitForCondition(() => menu.style.display != "none");
    word.click();
    await shownPromise;

    const items = [...menu.querySelectorAll("a")];
    ok(items.length >= 4, "There should be at least 4 items in the menu");

    const firstItem = items[0];
    const nextItem = items[1];

    const lastItem = items[items.length - 1];
    const lastPrevItem = items[items.length - 2];

    const tester = new KeyboardNavigationTester(menu);

    is(await tester.keydown("Down"), firstItem,
       "The first item should be focused when down key is pressed on the menu");
    is(await tester.keydown("Down"), nextItem,
       "The next item should be focused");

    is(await tester.keydown("Up"), firstItem,
       "The previous item should be focused");
    is(await tester.keydown("Up"), firstItem,
       "Moving up from the first item keeps the focus");

    is(await tester.keydown("ArrowDown"), nextItem,
       "The next item should be focused");

    is(await tester.keydown("ArrowUp"), firstItem,
       "The previous item should be focused");
    is(await tester.keydown("ArrowUp"), firstItem,
       "Moving up from the first item keeps the focus");

    is(await tester.keydown("PageDown"), lastItem,
       "The last item should be focused");

    is(await tester.keydown("Up"), lastPrevItem,
       "The previous item should be focused");
    is(await tester.keydown("Down"), lastItem,
       "The next item should be focused");
    is(await tester.keydown("Down"), lastItem,
       "Moving down from the last item keeps the focus");

    is(await tester.keydown("ArrowUp"), lastPrevItem,
       "The previous item should be focused");
    is(await tester.keydown("ArrowDown"), lastItem,
       "The next item should be focused");
    is(await tester.keydown("ArrowDown"), lastItem,
       "Moving down from the last item keeps the focus");

    is(await tester.keydown("PageUp"), firstItem,
       "The first item should be focused");

    is(await tester.keydown("End"), lastItem,
       "The last item should be focused");

    is(await tester.keydown("Home"), firstItem,
       "The first item should be focused");

    is(await tester.keydown("Left"), firstItem,
       "Moving left keeps the focus");
    is(await tester.keydown("ArrowLeft"), firstItem,
       "Moving left keeps the focus");
    is(await tester.keydown("Right"), firstItem,
       "Moving right keeps the focus");
    is(await tester.keydown("ArrowRight"), firstItem,
       "Moving right keeps the focus");

    const hidePromise = waitForCondition(() => menu.style.display == "none");
    TestUtils.keydown(tester.currentItem, { key: "Escape" });
    await hidePromise;
  }

  {
    const shownPromise = waitForCondition(() => menu.style.display != "none");
    word.click();
    await shownPromise;

    const items = [...menu.querySelectorAll("a")];
    const firstItem = items[0];

    const tester = new KeyboardNavigationTester(menu);

    is(await tester.keydown("ArrowDown"), firstItem,
       "The first item should be focused when down key is pressed on the menu");

    const hidePromise = waitForCondition(() => menu.style.display == "none");
    TestUtils.keydown(tester.currentItem, { key: "Esc" });
    await hidePromise;
  }

  {
    const shownPromise = waitForCondition(() => menu.style.display != "none");
    word.click();
    await shownPromise;

    const items = [...menu.querySelectorAll("a")];
    const lastItem = items[items.length - 1];

    const tester = new KeyboardNavigationTester(menu);

    is(await tester.keydown("Up"), lastItem,
       "The last item should be focused when down key is pressed on the menu");

    const hidePromise = waitForCondition(() => menu.style.display == "none");
    TestUtils.keydown(tester.currentItem, { key: "Escape" });
    await hidePromise;
  }

  {
    const shownPromise = waitForCondition(() => menu.style.display != "none");
    word.click();
    await shownPromise;

    const items = [...menu.querySelectorAll("a")];
    const lastItem = items[items.length - 1];

    const tester = new KeyboardNavigationTester(menu);

    is(await tester.keydown("ArrowUp"), lastItem,
       "The last item should be focused when down key is pressed on the menu");

    const hidePromise = waitForCondition(() => menu.style.display == "none");
    TestUtils.keydown(tester.currentItem, { key: "Esc" });
    await hidePromise;
  }
});
