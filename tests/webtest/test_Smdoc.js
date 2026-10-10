"use strict";

// SpiderMonkey's `[SMDOC]` citations in comments go to the blocks they cite.
add_task(async function test_SmdocCitation() {
  await TestUtils.loadPath("/tests/source/js/src/smdoc.cpp");

  const citation = frame.contentDocument.querySelector(
    `#line-16 span[data-symbols="SMDOC_Test@20Concept@20@28with@20context@29"]`);
  ok(citation, "The citation in the comment is a symbol");
  is(citation?.textContent, "Test Concept", "The citation is the words it shares with the title");
  ok(citation?.classList.contains("syn_comment"), "The citation is still styled as a comment");

  TestUtils.click(citation);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the citation");
  const item = [...menu.querySelectorAll("a")].find(
    a => a.textContent.includes("Go to documentation of Test Concept (with context)"));
  is(item?.getAttribute("href"), "/tests/source/js/src/smdoc.cpp#4",
     "The menu goes to the block's title");
  frame.contentWindow.ContextMenu.hide();
});
