"use strict";

add_task(async function test_TreeSwitcherKeyboardNavigation() {
  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  // Given the focus event requires user interaction, use test-only events
  // to detect the focus handling.
  const focusEvents = (async function* () {
    while (true) {
      const event = await new Promise(resolve => {
        frame.contentDocument.addEventListener("focusmenuitem", event => {
          resolve(event);
        }, { once: true });
      });

      yield event;
    }
  })();

  class KeyboardNavigationTester {
    constructor(currentItem) {
      this.currentItem = currentItem;

      // Set to true to track the navigation.
      this.debug = false;

      if (this.debug) {
        this.currentItem.style.outline = "1px dashed red";
      }
    }

    async keydown(key) {
      if (this.debug) {
        this.currentItem.style.outline = "";
      }

      const eventPromise = focusEvents.next();
      TestUtils.keydown(this.currentItem, { key });
      const event = (await eventPromise).value;
      this.currentItem = event.targetItem;

      if (this.debug) {
        this.currentItem.style.outline = "1px dashed red";
        await TestUtils.sleep(100);
      }

      return this.currentItem;
    }
  };

  const word = frame.contentDocument.querySelector(`span[data-symbols="T_webtest::SimpleSearch"]`);

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
