add_task(async function test_SpaceInFilenameInSearch() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "SymbolInFilenameWithSpace");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("Definitions (SymbolInFilenameWithSpace) (1 lines") &&
      content.textContent.includes("var SymbolInFilenameWithSpace"),
    "symbol in file with space in filename matches as definition");

  const links = frame.contentDocument.querySelectorAll(".result-head a");
  is(links.length, 2);
  is(links[1].getAttribute("href"), "/tests/source/js/with%20space.js",
     "The space in the href should be escaped");
});

add_task(async function test_SpaceInFilenameInFileView() {
  await TestUtils.loadPath("/tests/source/js/with%20space.js");

  const links = frame.contentDocument.querySelectorAll(".breadcrumbs a");

  is(links.length, 3);
  is(links[2].getAttribute("href"), "/tests/source/js/with%20space.js",
     "The space in the href should be escaped");
});

add_task(async function test_SpaceInFilenameInNavigationPanel() {
  await TestUtils.loadPath("/searchfox/source/tests/tests/files/js/with%20space.js");

  const panel = frame.contentDocument.getElementById("panel");
  const permalink = panel.querySelector(`.item[title="Permalink"]`);

  ok(permalink.getAttribute("href").includes("/js/with%20space.js"),
     "The space in the href should be escaped");
});

add_task(async function test_SpaceInFilenameInBlameAndOldRevision() {
  await TestUtils.loadPath("/searchfox/source/tests/tests/files/js/with%20space.js");

  // Test the blame popup

  const blameStrip = frame.contentDocument.querySelector(`#line-2 .blame-strip`);

  TestUtils.dispatchMouseEvent("mouseenter", blameStrip);

  function getLinks() {
    return frame.contentDocument.querySelectorAll(`#blame-popup a`);
  }

  await waitForCondition(() => getLinks().length > 0);

  const links = getLinks();
  is(links.length, 4);
  is(links[1].textContent, "annotated diff");
  ok(links[1].getAttribute("href").includes("/js/with%20space.js"),
     "The space in the href should be escaped");

  const annotatedDiffURL = links[1].href;

  is(links[2].textContent, "Show latest version without this line");
  ok(links[2].getAttribute("href").includes("/js/with%20space.js"),
     "The space in the href should be escaped");

  is(links[3].textContent, "Show earliest version with this line");
  ok(links[3].getAttribute("href").includes("/js/with%20space.js"),
     "The space in the href should be escaped");

  TestUtils.click(links[2]);

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("/searchfox/rev"),
    "Navigates to the previous version");

  // In order to avoid hard-coding the old version's hash, continue testing
  // the UI part of the old version page here.

  // Test breadcrumbs.
  {
    const links = frame.contentDocument.querySelectorAll(".breadcrumbs a");

    is(links.length, 6);
    is(links[5].getAttribute("href"), "/searchfox/source/tests/tests/files/js/with%20space.js",
       "The space in the href should be escaped");
  }

  // Test navigation panel.
  {
    const panel = frame.contentDocument.getElementById("panel");
    const goToLatestLink = panel.querySelector(`.item[title="Go to latest version"]`);

    ok(goToLatestLink.getAttribute("href").includes("/js/with%20space.js"),
       "The space in the href should be escaped");
  }
});

add_task(async function test_SpaceInFilenameInAnnotatedDiffAndChangeset() {
  await TestUtils.loadPath("/searchfox/source/tests/tests/files/js/with%20space.js");

  const blameStrip = frame.contentDocument.querySelector(`#line-2 .blame-strip`);

  TestUtils.dispatchMouseEvent("mouseenter", blameStrip);

  function getLinks() {
    return frame.contentDocument.querySelectorAll(`#blame-popup a`);
  }

  await waitForCondition(() => getLinks().length > 0);

  const links = getLinks();
  is(links.length, 4);
  is(links[1].textContent, "annotated diff");
  ok(links[1].getAttribute("href").includes("/js/with%20space.js"),
     "The space in the href should be escaped");

  TestUtils.click(links[1]);

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("/searchfox/diff"),
    "Navigates to the previous version");

  // Test breadcrumbs.
  {
    const links = frame.contentDocument.querySelectorAll(".breadcrumbs a");

    is(links.length, 6);
    is(links[5].getAttribute("href"), "/searchfox/source/tests/tests/files/js/with%20space.js",
       "The space in the href should be escaped");
  }

  // Test navigation panel.
  {
    const panel = frame.contentDocument.getElementById("panel");
    const goToLatestLink = panel.querySelector(`.item[title="Go to latest version"]`);

    ok(goToLatestLink.getAttribute("href").includes("/js/with%20space.js"),
       "The space in the href should be escaped");

    // In order to avoid hard-coding the old version's hash, continue testing
    // the UI part of the changeset view.

    const changesetLink = panel.querySelector(`.item[title="Show changeset"]`);

    TestUtils.click(changesetLink);
  }

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("/searchfox/commit"),
    "Navigates to the changeset view");

  // Test file listing.
  {
    const link = frame.contentDocument.querySelector(`#content ul li a[href*="space.js"]`);

    ok(link.getAttribute("href").includes("/js/with%20space.js"),
       "The space in the href should be escaped");
  }
});
