"use strict";

// The specpr tree's pull request summaries (see tools/src/spec_prs.rs), at
// `/specpr/spec-prs`, which its sync-spec-prs hook makes from a snapshot of
// their metadata standing for the GitHub Action's
// (tests/specpr/spec-prs-snapshot.json: pull request 1 is an editor's, and
// approved, 2 a contributor's draft, with changes requested, and 3 a
// collaborator's, needing review, 4 and 5 an editor's, labeled "meta", and 6
// a contributor's) and their heads (see tests/specpr/setup): 1 adds to the
// Wattsi spec's "Sprockets" and the Bikeshed spec's "Gears", 2 changes the
// Wattsi spec's "Gizmos", 3 the Bikeshed spec's "Gears", 4 and 5 add files
// which aren't specs, and 6 changes the file the ecmarkup spec imports.

function doc() {
  return frame.contentDocument;
}

async function synced() {
  await TestUtils.loadPath("/specpr/spec-prs");
  if (!doc().querySelector(".spec-prs-repos")) {
    ok(doc().querySelector(".spec-prs")?.textContent.includes("synced yet"),
       "Before a sync, there are no summaries");
    // (Unless the web server's start is syncing them already, when the page
    // reloads once it's done; see tools/src/spec_prs.rs's `start`.)
    doc().querySelector(".spec-prs-sync-button")?.click();
    // (Or says why it failed.)
    await waitForCondition(() => doc()?.querySelector(".spec-prs-repos, .spec-prs-error"),
                           "Syncing finishes", 500, 240);
    is(doc().querySelector(".spec-prs-error")?.textContent, undefined, "without failing");
  }
}

add_task(async function test_SpecPullRequestSummaries() {
  await synced();
  const cells = [...doc().querySelectorAll(".spec-prs-repos tbody tr:first-child td")]
    .map(cell => cell.textContent);
  is(cells.slice(0, 7).join(","), "gizmo,gizmo,8,1,1,6,1",
     "The specs' page lists the spec repository (its spec directory, then its repository), with its open, approved, changes requested, by people who can review and draft pull requests");
  is(doc().querySelector(".spec-prs-repo-page")?.getAttribute("href"), "/specpr/spec-prs/gizmo",
     "linking to its page");
  is([...doc().querySelectorAll(".breadcrumbs a")].map(a => a.textContent).join("/"), "specpr/Spec PRs",
     "Its breadcrumbs are the tree's and Spec PRs");
  const status = await (await fetch("/specpr/spec-prs/sync-status", { cache: "no-store" })).json();
  is(status.state, "idle", "Its sync's status says it's done");

  await TestUtils.loadPath("/specpr/spec-prs/gizmo");
  is([...doc().querySelectorAll(".breadcrumbs a")].map(a => a.getAttribute("href")).join(" "),
     "/specpr/source/ /specpr/spec-prs /specpr/spec-prs/gizmo",
     "The spec repository's page's breadcrumbs are the tree's, Spec PRs and the repository");
  ok(doc().querySelector("#panel-content .facet-box") && !doc().querySelector(".spec-prs .facet-box"),
     "Its facets are in the navigation panel");
  // (Its specs' views with all of their pull requests: of the Wattsi spec,
  // the Bikeshed spec's and the ecmarkup spec's, which their pull requests
  // change, the most changed first.)
  const rendered = [...doc().querySelectorAll(".spec-prs-rendered-link")];
  is(rendered.map(a => a.getAttribute("href")).join(" "),
     "/specpr/rendered-prs/gizmo/source /specpr/rendered-prs/gizmo/bs/index.bs /specpr/rendered-prs/gizmo/es/spec.html",
     "It links to its specs' renderings with all of their pull requests");
  ok(doc().querySelector(".spec-prs-rendered").textContent.startsWith("Rendered specs with all of their pull requests: gizmo ("),
     "naming them, with how many pull requests change each");
  const prs = [...doc().querySelectorAll(".spec-pr")];
  is(prs.map(pr => pr.dataset.path).join(","), "8,7,6,5,4,3,2,1", "and lists its pull requests, newest first");
  const [first, second] = ["1", "2"].map(number => prs.find(pr => pr.dataset.path == number));
  const title = first.querySelector(".spec-pr-title");
  is(title?.getAttribute("href"), "/specpr/try/gizmo/1", "linking to their pages as try pushes");
  ok(title?.textContent.includes("Define oiling sprockets, and springs"), "with their titles");
  is(first.querySelector(".spec-pr-review")?.textContent, "Approved", "and review decisions");
  ok(second.querySelector(".spec-pr-draft"), "and whether they're drafts");
  const sections = [...first.querySelectorAll(".spec-pr-files a")].map(a => a.getAttribute("href"));
  ok(sections.includes("/specpr/rendered/gizmo/source#sprockets") &&
     sections.includes("/specpr/rendered/gizmo/bs/index.bs#gears"),
     "and the sections they change, in the trunk's renderings");
  const imported = prs.find(pr => pr.dataset.path == "6");
  ok([...imported.querySelectorAll(".spec-pr-files a")].some(a => a.getAttribute("href") == "/specpr/rendered/gizmo/es/gauges.html#sec-gauges"),
     "including in files which ecmarkup specs import");
  is(imported.querySelector(".spec-pr-just-this")?.getAttribute("href"), "/specpr/rendered-prs/gizmo/es/gauges.html?prs=6",
     "which have views with just the pull request's changes too");
  is(imported.querySelector(".spec-pr-just-this")?.textContent, "with just this pull request", "so saying");

  // Filtering by review decision, and by whether the authors can review.
  const value = (facet, name) => [...doc().querySelectorAll(`.facet[data-facet="${facet}"] .facet-value`)]
    .find(button => button.textContent.includes(name));
  value("review", "Approved").click();
  is(prs.filter(pr => !pr.hidden).map(pr => pr.dataset.path).join(","), "1",
     "Facets filter them by review decision");
  is(doc().querySelector(".facet-status")?.textContent, "Showing 1 of 8 pull requests.",
     "counting pull requests");
  value("review", "Approved").click();
  value("author", "Other contributors").click();
  is(prs.filter(pr => !pr.hidden).map(pr => pr.dataset.path).join(","), "6,2",
     "and by whether their authors can review");
  doc().querySelector(".facet-clear").click();

  // Shift-clicking a value leaves its pull requests out instead (see
  // facets.js's `toggleValue`), and again, doesn't; clicking a left out value
  // selects it, and shift-clicking a selected one leaves it out.  The URL
  // says so (`-facet-review`), which reproduces it.
  const shiftClick = button => button.dispatchEvent(
    new frame.contentWindow.MouseEvent("click", { bubbles: true, shiftKey: true }));
  const shown = () => [...doc().querySelectorAll(".spec-pr")].filter(pr => !pr.hidden)
    .map(pr => pr.dataset.path).join(",");
  const approved = () => value("review", "Approved");
  shiftClick(approved());
  is(shown(), "8,7,6,5,4,3,2", "Shift-clicking a value leaves its pull requests out");
  ok(approved().classList.contains("facet-excluded"), "which the value shows");
  is(doc().querySelector(".facet-status")?.textContent, "Showing 7 of 8 pull requests.",
     "and the status counts");
  is(new URL(frame.contentWindow.location.href).searchParams.get("-facet-review"),
     approved().dataset.value, "as the URL says");
  shiftClick(approved());
  is(shown(), "8,7,6,5,4,3,2,1", "Shift-clicking it again doesn't leave them out");
  ok(!approved().classList.contains("facet-excluded"), "anymore");
  shiftClick(approved());
  approved().click();
  is(shown(), "1", "Clicking a left out value selects it");
  shiftClick(approved());
  is(shown(), "8,7,6,5,4,3,2", "and shift-clicking a selected one leaves it out");
  await TestUtils.loadPath(frame.contentWindow.location.pathname + frame.contentWindow.location.search);
  is(shown(), "8,7,6,5,4,3,2", "which the URL reproduces");
  ok(approved().classList.contains("facet-excluded"), "showing the value left out");
  doc().querySelector(".facet-clear").click();
  is(shown(), "8,7,6,5,4,3,2,1", "Showing them all again shows them all");

  // (And through the data API, which the web-server serves.)
  const api = await (await fetch("/specpr/api/v1/spec-prs?spec=gizmo", { cache: "no-store" })).json();
  is(api[0]?.prs?.length, 8, "The data API has the summaries");
});
