"use strict";

add_task(async function test_PanelOnLoad() {
  const tests = [
    {
      path: "/",
      expanded: false,
      empty: true,
    },
    {
      path: "/tests/pages/settings.html",
      expanded: false,
      empty: true,
    },
    {
      path: "/tests/source/",
      expanded: false,
      empty: true,
    },
    {
      path: "/tests/source/webtest",
      expanded: false,
      empty: true,
    },
    {
      path: "/tests/source/.gitignore",
      expanded: true,
      empty: false,
    },
    {
      path: "/tests/source/webtest/Webtest.cpp",
      expanded: true,
      empty: false,
    },
    {
      path: "/tests/search?q=webtest&path=&case=false&regexp=false",
      expanded: false,
      empty: true,
    },
    {
      path: "/tests/query/default?q=webtest",
      expanded: false,
      // Debug items are added on tests repository.
      empty: false,
    },
    {
      path: "/searchfox/diff/4e266f75295afe5f94d14eb9b72445c830c095ef/.eslintrc.js",
      expanded: true,
      empty: false,
    },
    {
      path: "/searchfox/commit/4e266f75295afe5f94d14eb9b72445c830c095ef",
      expanded: false,
      empty: true,
    },
    {
      path: "/searchfox/rev/e6ff7d3798a68e41c1166524be276fac4a8dfeb2/.gitignore",
      expanded: true,
      empty: false,
    },
  ];

  for (const { path, expanded, empty } of tests) {
    await TestUtils.loadPath(path);

    const panel = frame.contentDocument.querySelector("#panel");
    ok(!!panel, `Navigation panel node exists on ${path}`);

    const content = frame.contentDocument.querySelector("#panel-content");
    if (expanded) {
      is(content.getAttribute("aria-expanded"), "true",
         `Navigation panel is expanded on ${path}`);
    } else {
      is(content.getAttribute("aria-expanded"), "false",
         `Navigation panel is collapsed on ${path}`);
    }

    if (empty) {
      is(content.children.length, 1,
         `Navigation panel has only keyboard shortcut checkbox on ${path}`);
    } else {
      is(content.children.length > 1, true,
         `Navigation panel has multiple items on ${path}`);
    }
  }
});

add_task(async function test_PanelAfterSearch() {
  const tests = [
    {
      path: "/",
    },
    {
      path: "/tests/pages/settings.html",
    },
    {
      path: "/tests/source/",
    },
    {
      path: "/tests/source/webtest",
    },
    {
      path: "/tests/source/.gitignore",
    },
    {
      path: "/tests/source/webtest/Webtest.cpp",
    },
    {
      path: "/tests/search?q=webtest&path=&case=false&regexp=false",
    },
    {
      path: "/searchfox/diff/4e266f75295afe5f94d14eb9b72445c830c095ef/.eslintrc.js",
    },
    {
      path: "/searchfox/commit/4e266f75295afe5f94d14eb9b72445c830c095ef",
    },
    {
      path: "/searchfox/rev/e6ff7d3798a68e41c1166524be276fac4a8dfeb2/.gitignore",
    },
  ];

  for (const { path, expanded, empty } of tests) {
    await TestUtils.loadPath(path);
    TestUtils.shortenSearchTimeouts();

    const query = frame.contentDocument.querySelector("#query");
    TestUtils.setText(query, "SimpleSearch");

    const content = frame.contentDocument.querySelector("#content");
    await waitForCondition(
      () => content.textContent.includes("Number of results:"),
      "Search result is shown");

    const panel = frame.contentDocument.querySelector("#panel");
    ok(!!panel, `Navigation panel node exists on the search result from ${path}`);

    const panelContent = frame.contentDocument.querySelector("#panel-content");
    is(panelContent.getAttribute("aria-expanded"), "false",
       `Navigation panel is collapsed on the search result from ${path}`);
    is(panelContent.children.length, 1,
       `Navigation panel has only keyboard shortcut checkbox on the search result from ${path}`);
  }
});
