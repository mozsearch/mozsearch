"use strict";

// The specpr tree's pull request summaries (see tools/src/spec_prs.rs), at
// `/specpr/spec-prs`, which its sync-spec-prs hook makes from a snapshot of
// their metadata standing for the GitHub Action's
// (tests/specpr/spec-prs-snapshot.json: pull request 1 is an editor's, and
// approved, 2 a contributor's draft, with changes requested, and 3 a
// collaborator's, needing review) and their heads (see tests/specpr/setup): 1
// adds to the Wattsi spec's "Sprockets" and the Bikeshed spec's "Gears", 2
// changes the Wattsi spec's "Gizmos", and 3 the Bikeshed spec's "Gears".

function doc() {
  return frame.contentDocument;
}

async function synced() {
  await TestUtils.loadPath("/specpr/spec-prs");
  if (!doc().querySelector(".spec-prs-repos")) {
    ok(doc().querySelector(".spec-prs")?.textContent.includes("synced yet"),
       "Before a sync, there are no summaries");
    doc().querySelector(".spec-prs-sync-button").click();
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
  is(cells.slice(0, 6).join(","), "gizmo,3,1,1,2,1",
     "The specs' page lists the spec repository, with its open, approved, changes requested, by people who can review and draft pull requests");
  const status = await (await fetch("/specpr/spec-prs/sync-status", { cache: "no-store" })).json();
  is(status.state, "idle", "Its sync's status says it's done");

  await TestUtils.loadPath("/specpr/spec-prs/gizmo");
  is(doc().querySelector(".spec-prs h2")?.textContent.startsWith("Pull requests of gizmo"), true,
     "The spec repository's page is titled by it");
  const prs = [...doc().querySelectorAll(".spec-pr")];
  is(prs.map(pr => pr.dataset.path).join(","), "3,2,1", "and lists its pull requests, newest first");
  const [, second, first] = prs;
  const title = first.querySelector(".spec-pr-title");
  is(title?.getAttribute("href"), "/specpr/try/gizmo/1", "linking to their pages as try pushes");
  ok(title?.textContent.includes("Define oiling sprockets, and springs"), "with their titles");
  is(first.querySelector(".spec-pr-review")?.textContent, "Approved", "and review decisions");
  ok(second.querySelector(".spec-pr-draft"), "and whether they're drafts");
  const sections = [...first.querySelectorAll(".spec-pr-files a")].map(a => a.getAttribute("href"));
  ok(sections.includes("/specpr/rendered/gizmo/source#sprockets") &&
     sections.includes("/specpr/rendered/gizmo/bs/index.bs#gears"),
     "and the sections they change, in the trunk's renderings");

  // Filtering by review decision, and by whether the authors can review.
  const value = (facet, name) => [...doc().querySelectorAll(`.facet[data-facet="${facet}"] .facet-value`)]
    .find(button => button.textContent.includes(name));
  value("review", "Approved").click();
  is(prs.filter(pr => !pr.hidden).map(pr => pr.dataset.path).join(","), "1",
     "Facets filter them by review decision");
  is(doc().querySelector(".facet-status")?.textContent, "Showing 1 of 3 pull requests.",
     "counting pull requests");
  value("review", "Approved").click();
  value("author", "Other contributors").click();
  is(prs.filter(pr => !pr.hidden).map(pr => pr.dataset.path).join(","), "2",
     "and by whether their authors can review");
  doc().querySelector(".facet-clear").click();

  // (And through the data API, which the web-server serves.)
  const api = await (await fetch("/specpr/api/v1/spec-prs?spec=gizmo", { cache: "no-store" })).json();
  is(api[0]?.prs?.length, 3, "The data API has the summaries");
});
