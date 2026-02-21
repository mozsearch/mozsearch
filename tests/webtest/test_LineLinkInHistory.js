"use strict";

add_task(async function test_LineLinkInHistory() {
  const firstPath = "/tests/source/big_cpp.cpp#136";
  await TestUtils.loadPath(firstPath);

  const ThingDef = frame.contentDocument.querySelector("#line-136 .syn_def");

  await waitForCondition(() => ThingDef.getBoundingClientRect().y >= 0,
                         "The definition should become visible");

  const files = [
    "templates1.cpp",
    "templates2.cpp",
    "templates3.cpp",
    "templates4.cpp",
    "templates5.cpp",
    "templates6.cpp",
    "templates7.cpp",
    "templates1.cpp",
    "templates2.cpp",
    "templates3.cpp",
    "templates4.cpp",
    "templates5.cpp",
    "templates6.cpp",
    "templates7.cpp",
  ];

  // Navigate multiple times to make the first item no longer in bfcache, maybe.
  for (const file of files) {
    // Try to emulate a situation where there's a link to other file.
    const link = frame.contentDocument.createElement("a");
    link.href = file;
    link.append("Click me");
    frame.contentDocument.querySelector("#panel").append(link);
    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(link);
    await loadPromise;
  }

  // Go back to the first item.
  for (let i = 0; i < files.length; i++) {
    history.back();
  }

  await waitForCondition(() => {
    return frame.contentDocument.location.href.endsWith(firstPath) &&
      frame.contentDocument.querySelector("#line-136 .syn_def");
  }, "Went back to the first page.");

  // Ensure the line is still in the visible area.
  const ThingDef2 = frame.contentDocument.querySelector("#line-136 .syn_def");

  await waitForCondition(() => ThingDef2.getBoundingClientRect().y >= 0,
                         "The definition should become visible");
});
