"use strict";

add_task(async function test_StickyLines() {
  await TestUtils.loadPath("/tests/source/big_cpp.cpp#360");

  const breadcrumbs = frame.contentDocument.querySelector(".breadcrumbs");
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
     breadcrumbs.getBoundingClientRect().top,
     "the breadcrumbs should touch the bottom of the searchbox");
  is(breadcrumbs.getBoundingClientRect().bottom,
     sticky1.getBoundingClientRect().top,
     "the first sticky line should touch the bottom of the breadcrumb");
  is(sticky1.getBoundingClientRect().bottom,
     sticky2.getBoundingClientRect().top,
     "the second sticky line should touch the bottom of the first sticky line");
  is(sticky2.getBoundingClientRect().bottom,
     sticky3.getBoundingClientRect().top,
     "the second sticky line should touch the bottom of the first sticky line");
});

add_task(async function test_StickyLinesVarDecl() {
  await TestUtils.loadPath("/tests/source/cpp/nesting-initializer.cpp");

  const containers = frame.contentDocument.querySelectorAll(".nesting-container");
  is(containers.length, 5);

  is(containers[0].querySelector(".source-line-with-number").id, "line-1",
     "the namespace should have container");
  is(containers[1].querySelector(".source-line-with-number").id, "line-12",
     "long variable decl with a list should have container");
  is(containers[2].querySelector(".source-line-with-number").id, "line-26",
     "callLambda function should have container");
  is(containers[3].querySelector(".source-line-with-number").id, "line-30",
     "foo function should have container");
  is(containers[4].querySelector(".source-line-with-number").id, "line-40",
     "long variable decl with a function call should have container");

  frame.contentDocument.querySelector("#line-44").scrollIntoView();

  await waitForCondition(() => containers[0].firstChild.classList.contains("stuck"),
                         "namespace should stuck");
  ok(containers[3].firstChild.classList.contains("stuck"), "function should stuck");
  ok(containers[4].firstChild.classList.contains("stuck"), "varriable decl should stuck");
});
