"use strict";

add_task(async function test_StickyLines() {
  await TestUtils.loadPath("/tests/source/big_cpp.cpp#360");

  const sticky1 = frame.contentDocument.querySelector("#line-128");
  const sticky2 = frame.contentDocument.querySelector("#line-210");
  const sticky3 = frame.contentDocument.querySelector("#line-348");

  // The "stuck" class is added asynchronously.
  await waitForCondition(() => sticky1.classList.contains("stuck"),
                         "namespace should stuck");
  ok(sticky2.classList.contains("stuck"), "class should stuck");
  ok(sticky3.classList.contains("stuck"), "method should stuck");

  const searchbox = frame.contentDocument.querySelector("#search-box");
  is(searchbox.getBoundingClientRect().bottom,
     sticky1.getBoundingClientRect().top,
     "the first sticky line should touch the bottom of the search box");
  is(sticky1.getBoundingClientRect().bottom,
     sticky2.getBoundingClientRect().top,
     "the second sticky line should touch the bottom of the first sticky line");
  is(sticky2.getBoundingClientRect().bottom,
     sticky3.getBoundingClientRect().top,
     "the second sticky line should touch the bottom of the first sticky line");
});
