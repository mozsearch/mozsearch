"use strict";

add_task(async function test_TreeSwitcherKeyboardNavigation() {
  await TestUtils.loadPath("/searchfox/source/tests/tests/files/blame/blame-phab.txt");

  const blameStrip = frame.contentDocument.querySelector(`#line-1 .blame-strip`);

  TestUtils.dispatchMouseEvent("mouseenter", blameStrip);

  let link;
  await waitForCondition(() => {
    link = frame.contentDocument.querySelector(`#blame-popup a[href*="D12345"]`);
    return link;
  });

  is(link.href, "https://searchfox.org/D12345", "Phabricator revision link equals to the commit message");
});
