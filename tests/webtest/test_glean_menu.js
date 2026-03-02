add_task(async function test_glean_probe() {
  await TestUtils.loadPath("/tests/source/yaml/metrics.yaml");

  const span = frame.contentDocument.querySelector(`#line-6 span[data-symbols]`);

  TestUtils.click(span);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const links = [...menu.querySelectorAll("a")];
  const submenuItem = links.find(a => a.textContent == "Glean");
  ok(submenuItem, "The sub menu item exists");

  TestUtils.dispatchMouseEvent("mouseenter", submenuItem);

  await waitForCondition(() =>
    !!frame.contentDocument.querySelector(".context-submenu"),
    "sub menu is shown");

  const submenu = frame.contentDocument.querySelector(".context-submenu");

  const items = submenu.querySelectorAll("a");
  is(items.length, 3, "3 items are shown");

  is(items[1].href, "https://dictionary.telemetry.mozilla.org/apps/firefox_desktop/metrics/test_metrics_probe_one",
    "glean dictionary link is shown");
  is(items[2].href, "https://glam.telemetry.mozilla.org/fog/probe/test_metrics_probe_one/explore?",
     "GLAM link is shown");

  const clipboard = TestUtils.spyClipboard();

  TestUtils.click(items[0]);

  is(clipboard.value,
     "Glean.testMetrics.probeOne.testGetValue()",
     "about:glean expression is copied");
});
