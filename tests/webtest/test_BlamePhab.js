"use strict";

add_task(async function test_TestPhabricator() {
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

add_task(async function test_TestPullRequest() {
  await TestUtils.loadPath("/searchfox/source/tests/tests/files/blame/blame-pr.txt");

  const blameStrip = frame.contentDocument.querySelector(`#line-1 .blame-strip`);

  TestUtils.dispatchMouseEvent("mouseenter", blameStrip);

  let link;
  await waitForCondition(() => {
    link = frame.contentDocument.querySelector(`#blame-popup a[href*="900"]`);
    return link;
  });

  is(link.href, "https://github.com/mozsearch/mozsearch/pull/900", "Pull request link equals to the commit message");
});
