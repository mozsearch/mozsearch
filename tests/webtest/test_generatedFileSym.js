"use strict";

add_task(async function test_FileSymInBreadcrumb() {
  await TestUtils.loadPath("/tests/source/__GENERATED__/generated.cpp");

  const fileSymElem = frame.contentDocument.querySelector(".breadcrumbs span[data-symbols]");
  const syms = {}
  for (const sym of fileSymElem.getAttribute("data-symbols").split(",")) {
    syms[sym] = 1;
  }

  ok("FILE_win64@__GENERATED__/generated@2Ecpp" in syms,
     "win64 symbol should exist");
  ok("FILE_linux64-opt@__GENERATED__/generated@2Ecpp" in syms,
     "linux64-opt symbol should exist");
});

add_task(async function test_DiagramToGenerated() {
  await TestUtils.loadQuery("tests", "calls-to:'__GENERATED__/generated.h' depth:4");

  const generatedElem = frame.contentDocument.querySelector(`[data-symbols*="FILE_linux64-opt@__GENERATED__/generated@2Ecpp"]`);
  ok(generatedElem, "generated.cpp should be shown");
});

add_task(async function test_DiagramFromGenerated() {
  await TestUtils.loadQuery("tests", "calls-to:'nsISupports.h' depth:4 path-limit:100");

  const generatedElem = frame.contentDocument.querySelector(`[data-symbols*="FILE_linux64-opt@__GENERATED__/generated@2Ecpp"]`);
  ok(generatedElem, "generated.cpp should be shown");
});


