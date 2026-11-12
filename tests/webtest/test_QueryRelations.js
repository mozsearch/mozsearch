"use strict";

// /query/'s results of a method in an override hierarchy (see
// `ResultRelation` and tests/tests/files/overs.cpp): its own, and the uses of
// the methods it overrides (possible virtual calls of it) at first, with the
// rest (the methods it overrides, its overrides and its cousins) left out by
// the "Relation" facet, whose values are checkboxes, or for each method by
// the overview's buttons.  (The overrides of a method without a body, ex: an
// interface's, are left out too, as pure virtual methods' declarations are
// definitions in the analysis.)  And the overview of a class's inheritance
// hierarchy.

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
  const cousins = overview.querySelector("li.overview-cousin > details > summary");
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
  const base = [...frame.contentDocument.querySelectorAll(".panel-overview .overview-ancestor > .overview-row")]
    .find(row => row.querySelector(".overview-name")?.textContent == "SuperBase::teardown");
  ok(base?.querySelector(".overview-counts")?.textContent.includes("(2 from overrides)"),
     `as the overview says: ${base?.textContent}`);
  is(base.querySelector('.overview-toggle[data-relation="overridden"]')?.textContent, "definition, super calls",
     "whose button shows or leaves out its definition and the super calls");
});

// Groups' headings with their counts.
function groupCounts() {
  return Object.fromEntries(
    [...frame.contentDocument.querySelectorAll(".query-result details.facet-group")]
      .filter(group => !group.hidden)
      .map(group => group.querySelector(":scope > summary > h2"))
      .filter(heading => heading)
      .map(heading => heading.textContent.replace(/\s+/g, " ").trim())
      .map(text => [text.replace(/ \(\d+ lines? across \d+ files?\)$/, ""),
                    text.match(/\((\d+) lines? across/)?.[1]]));
}

// The overview's buttons show or leave out each method's results (a cousin's,
// ex: TripleSubTwo::triplePure, of TripleSubOne::triplePure's two cousins),
// and a row's all of its methods'; a relation with some of its methods'
// results left out is "mixed", as the Relation facet says, which a click on
// shows all of; and the URL says which are left out.
add_task(async function test_QueryOverviewMethods() {
  await TestUtils.loadQuery("tests", "TripleSubOne::triplePure");
  const doc = frame.contentDocument;
  const overview = doc.querySelector(".panel .panel-overview");
  const cousinsRow = overview.querySelector("li.overview-cousin > details");
  ok(cousinsRow?.querySelector("summary").textContent.startsWith("2 other overrides (cousins)"),
     `Its cousins are in a row: ${cousinsRow?.querySelector("summary").textContent}`);
  const rowButton = cousinsRow.querySelector(':scope > summary > .overview-toggle[data-relation="cousins"]');
  const cousinButton = name => [...cousinsRow.querySelectorAll(".overview-row")]
    .find(row => row.querySelector(".overview-name").textContent == name)
    ?.querySelector('.overview-toggle[data-relation="cousins"]');
  const two = cousinButton("TripleSubTwo::triplePure");
  ok(two, "with a button for each of them");
  is(two.getAttribute("aria-pressed"), "false", "whose results are left out at first");

  TestUtils.click(two);
  is(two.getAttribute("aria-pressed"), "true", "Its button shows its results");
  const counts = groupCounts();
  is(counts["Definitions of cousin overrides (TripleSubOne::triplePure)"], "1",
     `and not the other cousin's: ${JSON.stringify(counts)}`);
  is(counts["Uses of cousin overrides (TripleSubOne::triplePure)"], "1", "nor its uses");
  const shownCousinLines = [...doc.querySelectorAll('.query-result [data-syms]')]
    .filter(line => !line.hidden && !line.closest("[hidden]") &&
            line.closest("[data-facet-scope]").dataset.facetScope.includes("cousins"))
    .map(line => line.dataset.syms);
  ok(shownCousinLines.length && shownCousinLines.every(syms => syms == "_ZN12TripleSubTwo10triplePureEv"),
     `whose lines are only its: ${shownCousinLines}`);
  is(rowButton.getAttribute("aria-pressed"), "mixed", "The row's button says some are shown");
  is(relationValue("cousins").getAttribute("aria-pressed"), "mixed", "as does the Relation facet");
  is(cousinButton("TripleSubThree::triplePure").getAttribute("aria-pressed"), "false",
     "and the other cousin's button says it's left out");
  const left = new URL(doc.location.href).searchParams.getAll("-facet-relation");
  ok(left.includes("cousins|_ZN14TripleSubThree10triplePureEv") && left.includes("overridden"),
     `which the URL says: ${left}`);

  // (As links reproduce it.)
  await TestUtils.loadPath(frame.contentWindow.location.pathname + frame.contentWindow.location.search);
  is(groupCounts()["Definitions of cousin overrides (TripleSubOne::triplePure)"], "1",
     "A link to the results shows them so");
  is(relationValue("cousins").getAttribute("aria-pressed"), "mixed", "with the facet's value mixed");

  // (A click on the mixed value shows all of them.)
  TestUtils.click(relationValue("cousins"));
  is(groupCounts()["Definitions of cousin overrides (TripleSubOne::triplePure)"], "2",
     "Clicking the mixed value shows all of its results");
  const reloadedRow = frame.contentDocument.querySelector(
    '.panel-overview li.overview-cousin > details > summary > .overview-toggle[data-relation="cousins"]');
  is(reloadedRow.getAttribute("aria-pressed"), "true", "as the row's button says");
  TestUtils.click(reloadedRow);
  ok(!shownGroups().some(group => group.includes("cousin")), `which leaves them all out: ${shownGroups()}`);
  ok(relationValue("cousins").classList.contains("facet-excluded"), "as the facet's value says");

  // (The uses of the method it overrides, by its button.)
  const base = frame.contentDocument.querySelector(
    ".panel-overview .overview-ancestor > .overview-row .overview-toggle[data-relation=\"virtual-uses\"]");
  is(base?.getAttribute("aria-pressed"), "true", "The possible virtual calls show at first");
  TestUtils.click(base);
  ok(!shownGroups().some(group => group.startsWith("Possible virtual calls")),
     `until the method's uses' button leaves them out: ${shownGroups()}`);
});

// A class's overview: the classes it inherits from above it, and their other
// subclasses (its cousins) in a row of their own, linking to their results
// (which aren't the queried class's).
add_task(async function test_QueryOverviewClasses() {
  await TestUtils.loadQuery("tests", "'DoubleSubOne'");
  const overview = frame.contentDocument.querySelector(".panel .panel-overview");
  const names = [...overview.querySelectorAll(".overview-name")].map(a => a.textContent);
  is(names[0], "DoubleBase", `The class it inherits from is at the top: ${names}`);
  const queried = overview.querySelector(".overview-queried > .overview-row > .overview-name");
  is(queried?.textContent, "DoubleSubOne", "and the queried class below it");
  const cousins = overview.querySelector("li.overview-cousin > details");
  ok(cousins?.querySelector("summary").textContent.startsWith("1 other subclass (a cousin)"),
     `and its cousin in a row of its own: ${cousins?.querySelector("summary").textContent}`);
  const cousin = cousins.querySelector(".overview-name");
  is(cousin?.textContent, "DoubleSubTwo", "which lists it");
  ok(cousin.getAttribute("href").includes(encodeURIComponent("symbol:T_DoubleSubTwo")),
     "linking to its results");
  ok(!cousins.querySelector(".overview-toggle"), "which the queried class's results don't have");
});

// A class subclassed by more than the limit on subclasses says so, with a
// link to the results following them anyway.
add_task(async function test_QueryOverviewSubclassLimit() {
  await TestUtils.loadQuery("tests", "subclass-limit:1 'DoubleBase'");
  const overview = frame.contentDocument.querySelector(".panel .panel-overview");
  const note = overview.querySelector(".overview-queried > .overview-row > .overview-note");
  ok(note?.textContent.startsWith("subclassed by 2, too many to follow"),
     `The queried class says it has too many subclasses: ${note?.textContent}`);
  const anyway = note.querySelector("a");
  ok(anyway?.getAttribute("href").includes(encodeURIComponent("subclass-limit:0")),
     `with a link following them anyway: ${anyway?.getAttribute("href")}`);
});
