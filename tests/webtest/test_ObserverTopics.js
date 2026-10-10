"use strict";

// Observer service topics' uses, by literals or constants, are symbols, whose
// menus go to the functions notifying observers of them and the observers'
// methods receiving them.
add_task(async function test_ObserverTopicSlots() {
  await TestUtils.loadPath("/tests/source/observer/observer.cpp");

  // (Along with the macro's own symbol.)
  const use = frame.contentDocument.querySelector(
    `#line-40 span[data-symbols*="OBSTOPIC_test@2Dshutdown"]`);
  ok(use, "The notification's constant is a use of the topic");
  is(use?.textContent, "NS_TEST_SHUTDOWN_TOPIC", "The use is at the constant");

  TestUtils.click(use);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the topic");
  const items = [...menu.querySelectorAll("a")].map(a => [a.textContent, a.getAttribute("href")]);
  const item = text => items.find(([label]) => label.includes(text));
  ok(item("Go to definition of sender observer_test::Shutdown"),
     "The menu goes to the function notifying observers of the topic");
  ok(item("Go to definition of receiver observer_test::Observer::Observe"),
     "The menu goes to the observer's method receiving the topic");
  ok(!item("Go to definition of receiver observer_test::Observer::Register"),
     "Registering an observer doesn't receive the topic");
  frame.contentWindow.ContextMenu.hide();
});
