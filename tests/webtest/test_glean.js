"use strict";

add_task(async function test_glean_probe() {
  await TestUtils.loadPath("/tests/source/yaml/metrics.yaml");

  const span = frame.contentDocument.querySelector(`#line-6 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to definition of glean C++ member binding"));
  ok(gotoDef, "A menu item to jump to the C++ definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/source/yaml/bindings/TestMetrics.cpp",
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash,
     "#7",
     "Jumps to the definition line");
});

add_task(async function test_glean_extra() {
  await TestUtils.loadPath("/tests/source/yaml/metrics.yaml");

  const span = frame.contentDocument.querySelector(`#line-10 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to definition of glean C++ class binding"));
  ok(gotoDef, "A menu item to jump to the C++ definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/source/yaml/bindings/TestMetrics.h",
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash,
     "#9",
     "Jumps to the definition line");
});

add_task(async function test_glean_extra_field() {
  await TestUtils.loadPath("/tests/source/yaml/metrics.yaml");

  const span = frame.contentDocument.querySelector(`#line-11 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to definition of glean C++ member binding"));
  ok(gotoDef, "A menu item to jump to the C++ definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/source/yaml/bindings/TestMetrics.h",
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash,
     "#10",
     "Jumps to the definition line");
});

add_task(async function test_glean_cpp_probe() {
  await TestUtils.loadPath("/tests/source/yaml/bindings/TestMetrics.h");

  const span = frame.contentDocument.querySelector(`#line-7 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to Glean definition"));
  ok(gotoDef, "A menu item to jump to the Glean definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/source/yaml/metrics.yaml",
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash,
     "#6",
     "Jumps to the definition line");
});

add_task(async function test_glean_cpp_extra() {
  await TestUtils.loadPath("/tests/source/yaml/bindings/TestMetrics.h");

  const span = frame.contentDocument.querySelector(`#line-9 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to Glean definition"));
  ok(gotoDef, "A menu item to jump to the Glean definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/source/yaml/metrics.yaml",
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash,
     "#10",
     "Jumps to the definition line");
});

add_task(async function test_glean_cpp_extra_field() {
  await TestUtils.loadPath("/tests/source/yaml/bindings/TestMetrics.h");

  const span = frame.contentDocument.querySelector(`#line-10 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Go to Glean definition"));
  ok(gotoDef, "A menu item to jump to the Glean definition exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/source/yaml/metrics.yaml",
     "Jumps to the definition file");
  is(frame.contentDocument.location.hash,
     "#11",
     "Jumps to the definition line");
});

add_task(async function test_glean_js_probe() {
  await TestUtils.loadPath("/tests/source/yaml/glean.js");

  const span = frame.contentDocument.querySelector(`#line-2 span[data-symbols*="Glean.testMetrics#probeOne"]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const gotoDef = links.find(a => a.textContent.includes("Search for Glean member"));
  ok(gotoDef, "A menu item to search for Glean exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(gotoDef);
  await loadPromise;

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Definitions (1 lines"),
    "C++ definition is found");
  await waitForCondition(
    () => content.textContent.includes("Glean (1 lines"),
    "Glean definition is found");
});
