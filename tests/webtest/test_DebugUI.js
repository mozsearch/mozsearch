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

// The analysis records widget shows the hovered token's records.
add_task(async function test_AnalysisRecordsWidget() {
  await TestUtils.loadPath("/tests/source/webtest/Titler.cpp");
  const doc = frame.contentDocument;
  const content = doc.querySelector("#panel-content");
  if (content.style.display == "none") {
    TestUtils.click(doc.querySelector("#panel-toggle"));
  }
  const widget = doc.querySelector(".debug-records");
  ok(widget, "The Debug section has the analysis records widget");

  const token = doc.querySelector('#line-1 span[data-symbols="_Z14globalVariable"]');
  token.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
  await waitForCondition(
    () => widget.querySelector(".debug-records-title")?.textContent.startsWith("Line 1, columns 5-19"),
    "Hovering a token shows its records");
  const summaries = [...widget.querySelectorAll(".debug-record > summary")].map(s => s.textContent);
  ok(summaries.includes("def globalVariable"), "including its definition's");

  // Its properties can be hidden.
  TestUtils.click(widget.querySelector(".debug-records-header button"));
  const peekRange = [...widget.querySelectorAll(".debug-records-props label")]
    .find(l => l.textContent == "peekRange").querySelector("input");
  TestUtils.clickCheckbox(peekRange);
  ok(![...widget.querySelectorAll(".debug-records dt")].some(dt => dt.textContent == "peekRange"),
     "A hidden property isn't shown");
  TestUtils.clickCheckbox(peekRange);
});

add_task(async function test_QueryDebugLog() {
  await TestUtils.loadQuery("tests", "field-layout:'field_layout::holes::Sub'");

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
     TestUtils.getQueryPath("tests", "field-layout:'field_layout::holes::Sub'", { debug: "true" }),
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
     TestUtils.getQueryPath("tests", "field-layout:'field_layout::holes::Sub'"),
     "Query without debug log is opened");
});

add_task(async function test_QueryResultsJSON() {
  await TestUtils.loadQuery("tests", "field-layout:'field_layout::holes::Sub'");

  // The page only has a copy of the results when debugging, since it would
  // double big pages; otherwise the button fetches them.
  is(frame.contentWindow.QUERY_RESULTS_JSON, undefined,
     "The page doesn't have the results JSON");

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
  await waitForCondition(() => box.textContent.includes("SymbolTreeTableList"),
                         "results JSON is shown");

  TestUtils.click(resultsJSONButtton);

  ok(!TestUtils.isShown(box),
     "results JSON node is hidden");

  is(resultsJSONButtton.textContent, "Show results JSON",
     "Button text is updated");

  await TestUtils.loadQuery("tests", "field-layout:'field_layout::holes::Sub'",
                            { debug: "true" });
  ok(frame.contentWindow.QUERY_RESULTS_JSON?.SymbolTreeTableList,
     "When debugging, the page has the results JSON");
});
