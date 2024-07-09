"use strict";

add_task(async function test_BreadcrumbsOnLoad() {
  const tests = [
    {
      path: "/",
      hidden: true,
    },
    {
      path: "/tests/pages/settings.html",
      hidden: true,
    },
    {
      path: "/tests/source/",
      hidden: false,
      text: "tests",
    },
    {
      path: "/tests/source/webtest",
      hidden: false,
      text: "tests/webtest",
    },
    {
      path: "/tests/source/.gitignore",
      hidden: false,
      text: "tests/.gitignore",
    },
    {
      path: "/tests/source/webtest/Webtest.cpp",
      hidden: false,
      text: "tests/webtest/Webtest.cpp  (file symbol)",
    },
    {
      path: "/tests/search?q=webtest&path=&case=false&regexp=false",
      hidden: false,
      text: "tests",
    },
    {
      path: "/tests/query/default?q=webtest",
      hidden: false,
      text: "tests",
    },
    {
      path: "/searchfox/diff/4e266f75295afe5f94d14eb9b72445c830c095ef/.eslintrc.js",
      hidden: false,
      text: "searchfox/.eslintrc.js",
    },
    {
      path: "/searchfox/commit/4e266f75295afe5f94d14eb9b72445c830c095ef",
      hidden: false,
      text: "searchfox",
    },
    {
      path: "/searchfox/rev/e6ff7d3798a68e41c1166524be276fac4a8dfeb2/.gitignore",
      hidden: false,
      text: "searchfox/.gitignore",
    },
  ];

  for (const { path, hidden, text } of tests) {
    await TestUtils.loadPath(path);

    const breadcrumbs = frame.contentDocument.querySelector(".breadcrumbs");
    ok(!!breadcrumbs, `Breadcrumbs node exists on ${path}`);
    if (hidden) {
      is(breadcrumbs.style.display, "none", `Breadcrumbs is hidden on ${path}`);
    } else {
      isnot(breadcrumbs.style.display, "none", `Breadcrumbs is shown on ${path}`);
      is(breadcrumbs.textContent.trim(), text,
         `Breadcrumbs shows the correct path on ${path}`);
    }

    const treeSwitcher = breadcrumbs.querySelector("#tree-switcher");
    ok(!!treeSwitcher, `Tree switcher node exists on ${path}`);
    const treeSwitcherMenu = breadcrumbs.querySelector("#tree-switcher-menu");
    ok(!!treeSwitcherMenu, `Tree switcher menu node exists on ${path}`);
    is(treeSwitcherMenu.style.display, "none",
       `Tree switcher menu is hidden on ${path}`);

    if (!hidden) {
      TestUtils.click(treeSwitcher);

      waitForShown(treeSwitcherMenu,
                   `Tree switcher menu is shown after clicking switcher`);

      const href = frame.contentDocument.location.href;

      const loadPromise = TestUtils.waitForLoad();
      const links = treeSwitcherMenu.querySelectorAll("a[href]");
      is(links[0].textContent, "tests",
         "The first item should be tests");
      is(links[1].textContent, "searchfox",
         "The first item should be searchfox");
      const isTests = frame.contentDocument.location.href.includes("/tests/");
      if (isTests) {
        TestUtils.click(links[1]);
      } else {
        TestUtils.click(links[0]);
      }
      await loadPromise;

      if (isTests) {
        is(frame.contentDocument.location.href,
           href.replace(/tests/, "searchfox"),
           "Tree should be switched to searchfox");
      } else {
        is(frame.contentDocument.location.href,
           href.replace(/searchfox/, "tests"),
           "Tree should be switched to tests");
      }
    }
  }
});

add_task(async function test_BreadcrumbsAfterSearch() {
  // Search result is shown without navigation.
  // Breadcrumbs should be preserved across the search result display.

  const tests = [
    {
      path: "/",
      tree: "tests",
    },
    {
      path: "/tests/pages/settings.html",
      tree: "tests",
    },

    {
      path: "/tests/source/",
      tree: "tests",
    },
    {
      path: "/tests/source/webtest",
      tree: "tests",
    },
    {
      path: "/tests/source/.gitignore",
      tree: "tests",
    },
    {
      path: "/tests/source/webtest/Webtest.cpp",
      tree: "tests",
    },
    {
      path: "/tests/search?q=webtest&path=&case=false&regexp=false",
      tree: "tests",
    },

    {
      path: "/searchfox/diff/4e266f75295afe5f94d14eb9b72445c830c095ef/.eslintrc.js",
      tree: "searchfox",
    },
    {
      path: "/searchfox/commit/4e266f75295afe5f94d14eb9b72445c830c095ef",
      tree: "searchfox",
    },
    {
      path: "/searchfox/rev/e6ff7d3798a68e41c1166524be276fac4a8dfeb2/.gitignore",
      tree: "searchfox",
    },
  ];

  for (const { path, tree } of tests) {
    await TestUtils.loadPath(path);
    TestUtils.shortenSearchTimeouts();

    const query = frame.contentDocument.querySelector("#query");
    TestUtils.setText(query, "SimpleSearch");

    const panelContent = frame.contentDocument.querySelector("#content");
    await waitForCondition(
      () => panelContent.textContent.includes("Number of results:"),
      "Search result is shown");

    const breadcrumbs = frame.contentDocument.querySelector(".breadcrumbs");
    ok(!!breadcrumbs, `Breadcrumbs node exists on the search result from ${path}`);
    isnot(breadcrumbs.style.display, "none",
          `Breadcrumbs is shown on the search result from ${path}`);
    is(breadcrumbs.textContent.trim(), tree,
       `Breadcrumbs shows the tree name on the search result from ${path}`);

    const treeSwitcher = breadcrumbs.querySelector("#tree-switcher");
    ok(!!treeSwitcher, `Tree switcher node exists on the search result from ${path}`);
    const treeSwitcherMenu = breadcrumbs.querySelector("#tree-switcher-menu");
    ok(!!treeSwitcherMenu, `Tree switcher menu node exists on the search result from ${path}`);
  }
});
