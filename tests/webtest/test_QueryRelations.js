"use strict";

// /query/'s results of a method in an override hierarchy (see
// `ResultRelation` and tests/tests/files/overs.cpp): its own, and the uses of
// the methods it overrides (possible virtual calls of it) at first, with the
// rest (the methods it overrides, its overrides and its cousins) left out by
// the "Relation" facet, whose values are checkboxes.  (The overrides of a
// method without a body, ex: an interface's, are left out too, as pure
// virtual methods' declarations are definitions in the analysis.)

// The shown kind groups' headings (but textual occurrences', which have
// their own rules; see kind_group.liquid).
function shownGroups() {
  return [...frame.contentDocument.querySelectorAll(".query-result details.facet-group")]
    .filter(group => !group.hidden && !group.parentElement.closest("[hidden]"))
    .map(group => group.querySelector(":scope > summary > h2"))
    .filter(heading => heading)
    .map(heading => heading.textContent.replace(/\s+/g, " ").trim()
      .replace(/ \(\d+ lines? across \d+ files?\)$| \(\d+ lines?\)$/, ""))
    .filter(heading => !heading.startsWith("Textual Occurrences"));
}

function relationValue(value) {
  return frame.contentDocument.querySelector(
    `.panel .facet[data-facet="relation"] .facet-value[data-value="${value}"]`);
}

add_task(async function test_QueryRelations() {
  await TestUtils.loadQuery("tests", "DoubleSubOne::doublePure");
  const doc = frame.contentDocument;
  ok(doc.querySelector('.panel .facet[data-facet="relation"][data-toggles]'),
     "The results have a Relation facet of checkboxes");
  is(JSON.stringify(shownGroups()), JSON.stringify([
    "Definitions (DoubleSubOne::doublePure)",
    "Uses (DoubleSubOne::doublePure)",
    "Possible virtual calls (uses of DoubleBase::doublePure)",
  ]), "Its own results and the uses of what it overrides show at first");
  ok(relationValue("cousins")?.classList.contains("facet-excluded"),
     "and its cousins are left out");
  ok(relationValue("overridden")?.classList.contains("facet-excluded"),
     "as are the declarations of what it overrides");
  ok(!new URL(doc.location.href).searchParams.has("-facet-relation"),
     "which the URL doesn't need to say");
  const pathKindCounts = () => frame.contentDocument.querySelector(".query-result .query-counts")?.textContent;
  is(pathKindCounts(), " (4 lines across 3 files)", "The path kind's counts are of the shown groups");

  TestUtils.click(relationValue("cousins"));
  ok(shownGroups().includes("Definitions of cousin overrides (DoubleSubOne::doublePure)"),
     `Clicking cousins shows them: ${shownGroups()}`);
  ok(shownGroups().includes("Uses of cousin overrides (DoubleSubOne::doublePure)"),
     "and their uses");
  is(pathKindCounts(), " (6 lines across 5 files)", "which its path kind's counts then count");
  is(new URL(doc.location.href).searchParams.getAll("-facet-relation").join(), "overridden",
     "which the URL says");

  // (As links reproduce it.)
  await TestUtils.loadPath(frame.contentWindow.location.pathname + frame.contentWindow.location.search);
  ok(shownGroups().includes("Definitions of cousin overrides (DoubleSubOne::doublePure)"),
     "A link to the results shows them so");

  // (Showing all of them, the URL says none are left out.)
  TestUtils.click(frame.contentDocument.querySelector(".panel .facet-clear"));
  // (A pure virtual method's declaration is its definition.)
  ok(shownGroups().includes("Definitions of what it overrides (DoubleBase::doublePure)"),
     `Showing all shows the definitions of what it overrides: ${shownGroups()}`);
  is(new URL(frame.contentDocument.location.href).searchParams.getAll("-facet-relation").join("|"), "",
     "with an empty left out value");
  ok(new URL(frame.contentDocument.location.href).searchParams.has("-facet-relation"),
     "in the URL");
});

// A base method's overrides are left out at first, and shown by their
// Relation value.
add_task(async function test_QueryRelationsOverrides() {
  await TestUtils.loadQuery("tests", "DoubleBase::doublePure");
  is(JSON.stringify(shownGroups()), JSON.stringify([
    "Definitions (DoubleBase::doublePure)",
    "Uses (DoubleBase::doublePure)",
  ]), "A base method's own results show at first");
  TestUtils.click(relationValue("overrides"));
  ok(shownGroups().includes("Definitions of its overrides (DoubleBase::doublePure)"),
     `and its overrides', when shown: ${shownGroups()}`);
});

// The overview (the "overview" lens, on at first; see query-overview.js): the
// method the queried one overrides above it, linking to their own results,
// and its other override (the queried one's cousin) in a row of its own,
// whose button shows or leaves out the cousins' results, as the Relation
// facet does.
add_task(async function test_QueryOverview() {
  const OVERVIEW_KEY = "lens-results-overview";
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey(OVERVIEW_KEY));
  await TestUtils.loadQuery("tests", "DoubleSubOne::doublePure");
  const doc = frame.contentDocument;
  const win = frame.contentWindow;
  const lens = doc.querySelector('.panel .lens-toggle[data-lens="overview"]');
  ok(lens, "There's an overview lens");
  is(lens.getAttribute("aria-pressed"), "true", "which is on at first");
  const overview = doc.querySelector(".panel .panel-overview");
  isnot(win.getComputedStyle(overview).display, "none", "showing the overview");
  const names = [...overview.querySelectorAll(".overview-name")].map(a => a.textContent);
  is(names[0], "DoubleBase::doublePure", `with what it overrides at the top: ${names}`);
  const queried = overview.querySelector(".overview-queried > .overview-row > .overview-name");
  is(queried?.textContent, "DoubleSubOne::doublePure", "and the queried method below it");
  ok(queried.getAttribute("href").includes(encodeURIComponent("symbol:_ZN12DoubleSubOne10doublePureEv")),
     "linking to its own results");
  const cousins = overview.querySelector("li.overview-cousins > details > summary");
  ok(cousins?.textContent.startsWith("1 other override (a cousin)"),
     `and its cousin in a row of its own: ${cousins?.textContent}`);
  const button = cousins.querySelector('.overview-toggle[data-relation="cousins"]');
  is(button?.getAttribute("aria-pressed"), "false", "whose results are left out");
  TestUtils.click(button);
  ok(shownGroups().includes("Definitions of cousin overrides (DoubleSubOne::doublePure)"),
     "until its button shows them");
  is(button.getAttribute("aria-pressed"), "true", "as it then says");
  ok(!relationValue("cousins").classList.contains("facet-excluded"), "as does the Relation facet");
  TestUtils.click(relationValue("cousins"));
  is(button.getAttribute("aria-pressed"), "false", "whose changes it follows");

  TestUtils.click(lens);
  is(win.getComputedStyle(overview).display, "none", "The lens hides the overview");
  is(win.localStorage.getItem(win.storageKey(OVERVIEW_KEY)), "0", "for later pages of results too");
  win.localStorage.removeItem(win.storageKey(OVERVIEW_KEY));
});

// The uses of the method it overrides in the hierarchy's methods are calls
// of it from its overrides (super calls, ex: `SuperBase::teardown();` in
// SuperCousin's override), which aren't possible virtual calls of it, and
// are with what it overrides (see tests/tests/files/overs_super.cpp).
add_task(async function test_QueryRelationsSuperCalls() {
  await TestUtils.loadQuery("tests", "SuperMid::teardown");
  is(JSON.stringify(shownGroups()), JSON.stringify([
    "Definitions (SuperMid::teardown)",
    "Uses (SuperMid::teardown)",
    "Possible virtual calls (uses of SuperBase::teardown)",
  ]), "Its own results and the possible virtual calls show at first");
  const virtualCalls = [...frame.contentDocument.querySelectorAll(".query-result details.facet-group")]
    .find(group => group.querySelector(":scope > summary > h2")?.textContent.includes("Possible virtual calls"));
  is(virtualCalls?.querySelectorAll(".source-line-with-number").length, 1,
     "which are tearDownAll's call, not its overrides'");
  TestUtils.click(relationValue("overridden"));
  ok(shownGroups().includes("Super calls (uses of SuperBase::teardown in its overrides)"),
     `whose calls are with what it overrides: ${shownGroups()}`);
  const base = [...frame.contentDocument.querySelectorAll(".panel-overview .overview-overridden > .overview-row")]
    .find(row => row.querySelector(".overview-name")?.textContent == "SuperBase::teardown");
  ok(base?.querySelector(".overview-counts")?.textContent.includes("(2 from overrides)"),
     `as the overview says: ${base?.textContent}`);
});
