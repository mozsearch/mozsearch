"use strict";

// The /explore/ pages show the commits of bugs (or Phabricator revisions) and
// what they changed: a sparkline with a slot per commit for each file, and
// under it, its outline diff's definitions which changed.  Bug 1975601's 4
// parts landed together, so they share a commit time and are ordered by
// ancestry.
add_task(async function test_ExploreBug() {
  await TestUtils.loadPath("/searchfox/explore/bug/1975601");

  const doc = frame.contentDocument;
  const commits = [...doc.querySelectorAll(".explore-commit-summary")].map(h => h.textContent);
  is(commits.length, 4, "The bug has 4 commits");
  for (let i = 0; i < 4; i++) {
    ok(commits[i].includes(`Part ${i + 1}:`), `Part ${i + 1} is commit ${i + 1}`);
  }

  const files = [...doc.querySelectorAll(".explore-file")];
  const blameRs = files.find(f => f.querySelector(".explore-file-header a").textContent == "tools/src/blame.rs");
  ok(blameRs, "blame.rs has a sparkline");
  // With 4 commits, each commit's slot is 6px wide, and Part 1 is the first.
  ok(blameRs.querySelector('.explore-file-header rect[class^="explore-l"][x="0"]'),
     "Part 1 changed blame.rs");
  const row = [...blameRs.querySelectorAll(".explore-outline-rows tr")].find(
    r => r.querySelector(".explore-outline-name").textContent == "find_phab_rev");
  ok(row?.classList.contains("outline-added"), "The function Part 1 added is under it");
  is(row?.querySelector(".outline-badge").textContent, "added +63", "with its tokens");
  is(row?.querySelector(".outline-diff-commit").getAttribute("href"), "#commit-1",
     "and the commit which added it");
  ok(/^\/searchfox\/rev\/[0-9a-f]{40}\/tools\/src\/blame\.rs#\d+$/.test(
       row?.querySelector(".explore-outline-name a").getAttribute("href")),
     "which links to its lines");

  // Hovering over a slot shows its commit.
  const svg = blameRs.querySelector(".explore-file-header svg");
  const rect = svg.getBoundingClientRect();
  svg.dispatchEvent(new MouseEvent("mousemove", {
    bubbles: true,
    clientX: rect.left + 1,
    clientY: rect.top + 1,
  }));
  ok(doc.querySelector(".explore-status").textContent.includes("Commit 1:"),
     "The status says which commit is under the mouse");
  ok(doc.querySelector("#commit-1").classList.contains("explore-current"),
     "The commit is highlighted in the list");
});

// The definitions which changed are nested in the ones they're in (as on the
// outline diffs' pages), which are shown unchanged, and the files' sparklines
// are faded.
add_task(async function test_ExploreOutlines() {
  await TestUtils.loadPath("/searchfox/explore/bug/2002694");

  const doc = frame.contentDocument;
  const file = doc.querySelector('.explore-file[data-path="clang-plugin/MozsearchIndexer.cpp"]');
  const rows = [...file.querySelectorAll(".explore-outline-rows tr")];
  const name = row => row.querySelector(".explore-outline-name").textContent;
  const depth = row => row.style.getPropertyValue("--outline-depth");
  is(name(rows[0]), "IndexConsumer", "The class the changed methods are in is first");
  ok(rows[0].matches(".outline-unchanged.outline-context"), "unchanged");
  const members = rows.slice(1);
  is(members.length, 7, "Its changed methods follow");
  ok(members.every(row => row.matches(".outline-changed") && depth(row) == "1"), "nested in it");
  const combine = members.find(row => name(row) == "combineRanges");
  ok(combine, "without the class's prefix");
  is(combine?.querySelector(".outline-badge").textContent, "+4 \u22124", "with their tokens");
  ok(file.querySelector(".explore-outline").getAttribute("href").includes("/outline-diff/"),
     "The file links to its whole outline diff");
  ok(Number(getComputedStyle(file.querySelector(".explore-file-header svg")).opacity) < 1,
     "Its sparkline is faded");
  is(file.dataset.size, "32", "Its size is its outline diff's tokens added and removed");
});

add_task(async function test_ExploreMissingBug() {
  await TestUtils.loadPath("/searchfox/explore/bug/9999999");
  ok(frame.contentDocument.querySelector(".explore-note").textContent.includes("No commits mention 9999999"),
     "The page says when a bug has no commits");
});

// The files are grouped by path kind (core code first, as on /search/), and
// facets filter them by path kind, subsystem, and directory, or group them by
// another facet.  Bug 2018468 changed the diagram code and its webtests.
add_task(async function test_ExploreFacets() {
  await TestUtils.loadPath("/searchfox/explore/bug/2018468");

  const doc = frame.contentDocument;
  const headers = () => [...doc.querySelectorAll(".facet-group")]
    .filter(group => !group.hidden)
    .map(group => group.querySelector(".facet-group-header").textContent);
  const shown = () => [...doc.querySelectorAll(".explore-file")]
    .filter(file => !file.hidden)
    .map(file => file.dataset.path);
  const all = shown();
  const toggleTop = () => doc.querySelector(".facet-toggle").getBoundingClientRect().top;
  const top = toggleTop();
  ok(headers()[0].startsWith("Core code") && headers()[1].startsWith("Test files"),
     "The files are grouped by path kind, core code first");

  const test = doc.querySelector('.facet[data-facet="kind"] .facet-value[data-value="test"]');
  ok(test, "There's a path kind facet");
  TestUtils.click(test);
  await waitForCondition(() => shown().length < all.length, "Selecting test files filters the files");
  ok(shown().every(path => path.startsWith("tests/")), "to the test files");
  is(toggleTop(), top, "The facets' header doesn't move when it says files are hidden");
  is(test.getAttribute("aria-pressed"), "true", "The test files are selected");
  ok(frame.contentWindow.location.search.includes("facet-kind=test"), "The selection is in the URL");
  const others = [...doc.querySelectorAll('.facet[data-facet="dir"] .facet-value')]
    .filter(value => /^(static|tools)\//.test(value.dataset.value));
  ok(others.length > 0 && others.every(value => value.querySelector(".facet-count").textContent == "0"),
     "The directories' counts are of the test files");

  TestUtils.selectMenu(doc.querySelector(".facet-group-by select"), "dir");
  await waitForCondition(() => headers().every(header => header.startsWith("tests/")),
                         "The test files are grouped by directory");
  ok(frame.contentWindow.location.search.includes("group-by=dir"), "The grouping is in the URL");

  TestUtils.click(doc.querySelector(".facet-clear"));
  await waitForCondition(() => shown().length == all.length, "Clearing the selection shows all the files");
});

add_task(async function test_ExploreFacetsFromURL() {
  await TestUtils.loadPath("/searchfox/explore/bug/2018468?facet-kind=test&group-by=dir");

  const doc = frame.contentDocument;
  const shown = [...doc.querySelectorAll(".explore-file")].filter(file => !file.hidden);
  ok(shown.length > 0 && shown.every(file => file.dataset.path.startsWith("tests/")),
     "The URL's selection filters the files");
  is(doc.querySelector(".facet-group-by select").value, "dir", "and groups them");
  ok(doc.querySelector(".facet-status").textContent.startsWith(`Showing ${shown.length} of`),
     "The status says how many files are shown");
});

// The files can be sorted by how many tokens they changed, biggest first,
// and the groups (but path kinds, whose order is their own) by their biggest
// files'.
add_task(async function test_ExploreSortBy() {
  await TestUtils.loadPath("/searchfox/explore/bug/2018468");

  const doc = frame.contentDocument;
  const headers = () => [...doc.querySelectorAll(".facet-group-header")]
    .map(header => header.firstChild.textContent.trim());
  const sizes = group => [...group.querySelectorAll(".explore-file")]
    .map(file => Number(file.dataset.size));
  const sorted = values => values.every((value, i) => i == 0 || values[i - 1] >= value);
  is(doc.querySelector(".facet-sort-by select").value, "alpha", "The files are sorted by path");

  TestUtils.selectMenu(doc.querySelector(".facet-sort-by select"), "size");
  await waitForCondition(() => frame.contentWindow.location.search.includes("sort-by=size"),
                         "The sorting is in the URL");
  ok([...doc.querySelectorAll(".facet-group")].every(group => sorted(sizes(group))),
     "The files are sorted by size, biggest first");
  is(doc.querySelector(".explore-file").dataset.path, "static/js/search.js", "the biggest first");
  ok(headers()[0].startsWith("Core code") && headers()[1].startsWith("Test files"),
     "The path kinds stay in their order");

  TestUtils.selectMenu(doc.querySelector(".facet-group-by select"), "dir");
  await waitForCondition(() => headers()[0] == "static/js/", "The directories are sorted by size");
  const maxima = [...doc.querySelectorAll(".facet-group")].map(group => Math.max(...sizes(group)));
  ok(sorted(maxima), `by their biggest files': ${JSON.stringify(headers())}`);

  TestUtils.selectMenu(doc.querySelector(".facet-sort-by select"), "alpha");
  await waitForCondition(() => headers()[0] == "static/css/", "and back by name");
});

// The groups' headers' context menu chooses the grouping and the sorting too.
add_task(async function test_ExploreGroupHeaderMenu() {
  await TestUtils.loadPath("/searchfox/explore/bug/2018468");

  const doc = frame.contentDocument;
  const header = doc.querySelector(".facet-group-header");
  const right = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 });
  ok(!header.dispatchEvent(right), "A group's header's right-click is the page's");
  const menu = doc.querySelector("#context-menu");
  await waitForShown(menu, "with a menu");
  const labels = [...menu.querySelectorAll(".submenu-label")];
  is(labels.map(label => label.textContent).join(", "), "Group by, Sort by",
     "of the choices of grouping and sorting");

  TestUtils.dispatchMouseEvent("mouseenter", labels[0]);
  await waitForCondition(() => doc.querySelector(".context-submenu"), "Which are submenus");
  const submenu = doc.querySelector(".context-submenu");
  await waitForShown(submenu, "shown on hover");
  const checked = [...submenu.querySelectorAll("a:not(.facet-choice-off)")].map(a => a.textContent);
  is(checked.join(), "Path kind", "checking the current choice");
  TestUtils.click([...submenu.querySelectorAll("a")].find(a => a.textContent == "Directory"));
  await waitForCondition(() => doc.querySelector(".facet-group-by select").value == "dir",
                         "Choosing another groups the files by it");
  ok(doc.querySelector(".facet-group-header").textContent.startsWith("static/css/"),
     "by directory");
});

// The facets are in a collapsible box, which stays collapsed.
add_task(async function test_ExploreFacetsCollapse() {
  registerCleanupFunction(() => frame.contentWindow.localStorage.removeItem("facets-collapsed"));
  await TestUtils.loadPath("/searchfox/explore/bug/2018468?facet-kind=test");

  const doc = frame.contentDocument;
  const toggle = doc.querySelector(".facet-toggle");
  is(toggle.getAttribute("aria-expanded"), "true", "The facets are shown");
  TestUtils.click(toggle);
  await waitForCondition(() => doc.querySelector(".facet-bar").hidden, "Clicking the toggle hides the facets");
  is(toggle.getAttribute("aria-expanded"), "false", "and says so");
  ok(TestUtils.isShown(doc.querySelector(".facet-status")), "The status still says that files are hidden");

  await TestUtils.loadPath("/searchfox/explore/bug/2018468");
  ok(frame.contentDocument.querySelector(".facet-bar").hidden, "The facets stay hidden");
});
