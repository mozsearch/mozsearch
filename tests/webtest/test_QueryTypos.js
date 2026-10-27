"use strict";

// A query which finds nothing suggests corrections of its typos, from the
// words the index knows (its identifiers' names and its files' and
// directories'; see tools/src/typos.rs): `bog_cpp.cpp` for big_cpp.cpp, whose
// "o" is next to the "i" on the keyboard.
add_task(async function test_QueryTypos() {
  await TestUtils.loadQuery("tests", "bog_cpp.cpp");
  const link = frame.contentDocument.querySelector(".query-corrections a");
  is(link?.textContent, "big_cpp.cpp", "A query finding nothing suggests a correction of its typo");
  await TestUtils.loadPath(link.getAttribute("href"));
  ok(!frame.contentDocument.querySelector(".query-corrections"), "whose query finds something");
  ok(frame.contentDocument.querySelector(".query-result")?.textContent.includes("big_cpp.cpp"),
     "the file");

  await TestUtils.loadQuery("tests", "big_cpp.cpp");
  ok(!frame.contentDocument.querySelector(".query-corrections"),
     "A query finding something suggests nothing");
});
