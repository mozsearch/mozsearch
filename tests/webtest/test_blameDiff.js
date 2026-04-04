"use strict";

add_task(async function test_TestPhabricator() {
  await TestUtils.loadPath("/searchfox/diff/b17c096ff1eab51aaf27befb5bd97ead09c74110/.gitignore");

  {
    const blameStrip = frame.contentDocument.querySelector(`#line-1 .blame-strip`);
    ok(blameStrip.getBoundingClientRect().height > 0,
       "Blame strip is visible for existing line");
  }

  {
    const blameStrip = frame.contentDocument.querySelector(`#line-7 .blame-strip`);
    ok(blameStrip.getBoundingClientRect().height > 0,
       "Blame strip is visible for newly added line");
  }
});
