"use strict";

add_task(async function test_TreeSwitcherKeyboardNavigation() {
  await TestUtils.loadPath("/tests/source/");

  // Use minimal tree list.
  frame.contentWindow.TREE_LIST = [
    [
      {
        name: "Test",
        items: [
          { value: "tests" },
          { value: "searchfox" },
        ],
      },
      {
        name: "Firefox",
        items: [
          { value: "mozilla-central" },
          { value: "mozilla-beta" },
          { value: "mozilla-release" },
        ],
      },
    ],
    [
      {
        name: "Firefox other",
        items: [
          { value: "mozilla-mobile" },
        ],
      },
      {
        name: "Thunderbird",
        items: [
          { value: "comm-central" },
        ],
      },
    ],
    [
      {
        name: "MinGW",
        items: [
          { value: "mingw" },
          { value: "mingw-moz" },
        ],
      },
      {
        name: "Other",
        items: [
          { value: "wubkat" },
        ]
      }
    ]
  ];

  const breadcrumbs = frame.contentDocument.querySelector(".breadcrumbs");

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

      return this.currentItem.textContent;
    }
  };

  const treeSwitcher = breadcrumbs.querySelector("#tree-switcher");
  const treeSwitcherMenu = breadcrumbs.querySelector("#tree-switcher-menu");

  const eventPromise = focusEvents.next();
  const shownPromise = waitForCondition(() => treeSwitcherMenu.style.display == "flex");
  treeSwitcher.click();
  await shownPromise;

  const event = (await eventPromise).value;
  is(event.targetItem.textContent, "tests",
     "The current tree should be focused");

  const tester = new KeyboardNavigationTester(event.targetItem);

  is(await tester.keydown("Down"), "searchfox",
     "The next tree should be focused");
  is(await tester.keydown("Down"), "mozilla-central",
     "The next tree should be focused, skipping the group label");

  is(await tester.keydown("Up"), "searchfox",
     "The previous tree should be focused, skipping the group label");
  is(await tester.keydown("Up"), "tests",
     "The previous tree should be focused");

  is(await tester.keydown("Up"), "tests",
     "Moving up from the first item keeps the focus");

  is(await tester.keydown("ArrowDown"), "searchfox",
     "The next tree should be focused");
  is(await tester.keydown("ArrowUp"), "tests",
     "The previous tree should be focused");

  is(await tester.keydown("Right"), "mozilla-mobile",
     "The next column's tree should be focused");
  is(await tester.keydown("Right"), "mingw",
     "The next column's tree should be focused");
  is(await tester.keydown("Right"), "mingw",
     "Moving right from the last column keeps the focus");

  is(await tester.keydown("Left"), "mozilla-mobile",
     "The previous column's tree should be focused");
  is(await tester.keydown("Left"), "tests",
     "The previous column's tree should be focused");

  is(await tester.keydown("Left"), "tests",
     "Moving left from the first column keeps the focus");

  is(await tester.keydown("ArrowRight"), "mozilla-mobile",
     "The next column's tree should be focused");
  is(await tester.keydown("ArrowLeft"), "tests",
     "The previous column's tree should be focused");

  is(await tester.keydown("ArrowLeft"), "tests",
     "The previous column's tree should be focused");

  is(await tester.keydown("Down"), "searchfox",
     "The next tree should be focused");
  is(await tester.keydown("Right"), "comm-central",
     "The next column's tree should be focused, skipping the group label to down");
  is(await tester.keydown("Right"), "wubkat",
     "The next column's tree should be focused, skipping the group label to down");
  is(await tester.keydown("Up"), "mingw-moz",
     "The previous tree should be focused, skipping the group label");
  is(await tester.keydown("Left"), "comm-central",
     "The previous column's tree should be focused, skipping the group label to down");
  is(await tester.keydown("Left"), "mozilla-central",
     "The previous column's tree should be focused, skipping the group label to down");

  is(await tester.keydown("PageDown"), "mozilla-release",
     "The last tree in the column should be focused");
  is(await tester.keydown("Down"), "mozilla-mobile",
     "The first tree in the next column should be focused");
  is(await tester.keydown("PageDown"), "comm-central",
     "The last tree in the column should be focused");
  is(await tester.keydown("Down"), "mingw",
     "The first tree in the next column should be focused");
  is(await tester.keydown("PageDown"), "wubkat",
     "The last tree in the column should be focused");
  is(await tester.keydown("Down"), "wubkat",
     "Moving down from the last item keeps the focus");

  is(await tester.keydown("PageUp"), "mingw",
     "The first tree in the column should be focused");
  is(await tester.keydown("Up"), "comm-central",
     "The last tree in the previous column should be focused");
  is(await tester.keydown("PageUp"), "mozilla-mobile",
     "The first tree in the column should be focused");
  is(await tester.keydown("Up"), "mozilla-release",
     "The last tree in the previous column should be focused");
  is(await tester.keydown("PageUp"), "tests",
     "The last tree in the column should be focused");

  is(await tester.keydown("End"), "wubkat",
     "The last tree in the last column should be focused");
  is(await tester.keydown("Home"), "tests",
     "The first tree in the first column should be focused");

  const hidePromise = waitForCondition(() => treeSwitcherMenu.style.display == "none");
  TestUtils.keydown(tester.currentItem, { key: "Escape" });
  await hidePromise;

  {
    const shownPromise = waitForCondition(() => treeSwitcherMenu.style.display == "flex");
    treeSwitcher.click();
    await shownPromise;

    const hidePromise = waitForCondition(() => treeSwitcherMenu.style.display == "none");
    TestUtils.keydown(tester.currentItem, { key: "Esc" });
    await hidePromise;
  }
});
