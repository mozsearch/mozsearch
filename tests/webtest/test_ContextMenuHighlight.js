"use strict";

add_task(async function test_highlight_update_with_menu_open() {
  await TestUtils.loadPath("/tests/source/cpp/gc.cpp");

  const tokens = frame.contentDocument.querySelectorAll("span[data-symbols]");
  const visibleTokens = Array.from(tokens).filter(t => t.textContent.trim().length > 0);

  ok(visibleTokens.length >= 2, "Found enough tokens to run the test");
  
  const token1 = visibleTokens[0];
  const token2 = visibleTokens[1];

  // 1. Click the first token to open the menu
  TestUtils.click(token1);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown");
  
  ok(token1.classList.contains("hovered"), "Token 1 should be highlighted initially");

  // 2. Click the second token *without* closing the menu
  // This verifies the fix for Bug 1761840 where mousemove suppression prevented updates
  TestUtils.click(token2);

  ok(token2.classList.contains("hovered"), "Token 2 should be highlighted after clicking it");
  ok(!token1.classList.contains("hovered"), "Token 1 should no longer be highlighted");
});
