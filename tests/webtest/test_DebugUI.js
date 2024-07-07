"use strict";

add_task(async function test_RawAnalysisLink() {
  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  let rawAnalysisLink = null;
  const panelContent = frame.contentDocument.querySelector("#panel-content");
  for (const link of panelContent.querySelectorAll("a")) {
    if (link.textContent == "Raw analysis records") {
      rawAnalysisLink = link;
      break;
    }
  }
  ok(rawAnalysisLink, "Raw analysis records link exists");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(rawAnalysisLink);
  await loadPromise;

  is(frame.contentDocument.location.pathname,
     "/tests/raw-analysis/webtest/Webtest.cpp",
     "Raw analysis page is opened");
});

add_task(async function test_QueryDebugLog() {
  await TestUtils.loadPath("/tests/query/default?q=field-layout%3A%27field_layout%3A%3Aholes%3A%3ASub%27");

  {
    const logs = frame.contentDocument.querySelector("#query-debug-logs");
    ok(!logs, "Debug logs are not shown by default");

    let debugLogsLink = null;
    const panelContent = frame.contentDocument.querySelector("#panel-content");
    for (const link of panelContent.querySelectorAll("a")) {
      if (link.textContent == "Show debug log") {
        debugLogsLink = link;
        break;
      }
    }
    ok(debugLogsLink, "Debug log link exists");

    is(panelContent.getAttribute("aria-expanded"), "false",
       `Navigation panel is collapsed`);
    const toggle = frame.contentDocument.querySelector("#panel-toggle");
    TestUtils.click(toggle);
    is(panelContent.getAttribute("aria-expanded"), "true",
       `Navigation panel is expanded`);

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(debugLogsLink);
    await loadPromise;
  }

  is(frame.contentDocument.location.pathname + frame.contentDocument.location.search,
     "/tests/query/default?q=field-layout%3A%27field_layout%3A%3Aholes%3A%3ASub%27&debug=true",
     "Query with debug log is opened");

  {
    const logs = frame.contentDocument.querySelector("#query-debug-logs");
    ok(!!logs, "Debug logs are shown");
    ok(logs.textContent.includes("logged_span"),
       "log JSON is shown");

    let debugLogsLink = null;
    const panelContent = frame.contentDocument.querySelector("#panel-content");
    for (const link of panelContent.querySelectorAll("a")) {
      if (link.textContent == "Hide debug log") {
        debugLogsLink = link;
        break;
      }
    }
    ok(debugLogsLink, "Debug log link exists");

    is(panelContent.getAttribute("aria-expanded"), "false",
       `Navigation panel is collapsed`);
    const toggle = frame.contentDocument.querySelector("#panel-toggle");
    TestUtils.click(toggle);
    is(panelContent.getAttribute("aria-expanded"), "true",
       `Navigation panel is expanded`);

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(debugLogsLink);
    await loadPromise;
  }

  is(frame.contentDocument.location.pathname + frame.contentDocument.location.search,
     "/tests/query/default?q=field-layout%3A%27field_layout%3A%3Aholes%3A%3ASub%27",
     "Query without debug log is opened");
});

add_task(async function test_QueryResultsJSON() {
  await TestUtils.loadPath("/tests/query/default?q=field-layout%3A%27field_layout%3A%3Aholes%3A%3ASub%27");

  const box = frame.contentDocument.querySelector("#query-debug-results-json");
  ok(!!box, "results JSON node exists");
  ok(!TestUtils.isShown(box),
     "results JSON node is hidden by default");

  let resultsJSONButtton = null;
  const panelContent = frame.contentDocument.querySelector("#panel-content");
  for (const button of panelContent.querySelectorAll("button")) {
    if (button.textContent == "Show results JSON") {
      resultsJSONButtton = button;
      break;
    }
  }
  ok(resultsJSONButtton, "Results JSON button exists");

  is(panelContent.getAttribute("aria-expanded"), "false",
     `Navigation panel is collapsed`);
  const toggle = frame.contentDocument.querySelector("#panel-toggle");
  TestUtils.click(toggle);
  is(panelContent.getAttribute("aria-expanded"), "true",
     `Navigation panel is expanded`);

  TestUtils.click(resultsJSONButtton);

  is(resultsJSONButtton.textContent, "Hide results JSON",
     "Button text is updated");

  ok(TestUtils.isShown(box),
     "results JSON node is shown");
  ok(box.textContent.includes("SymbolTreeTableList"),
     "results JSON is shown");

  TestUtils.click(resultsJSONButtton);

  ok(!TestUtils.isShown(box),
     "results JSON node is hidden");

  is(resultsJSONButtton.textContent, "Show results JSON",
     "Button text is updated");
});
