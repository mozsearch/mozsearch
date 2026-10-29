"use strict";

// A spec repository's pull requests' imports from its summaries' page (see
// tools/src/spec_prs.rs's `pr_imports` and tools/src/try_import.rs's
// `request_all`): it says which are imported, and its button imports those
// the facets show which need it, a few at a time.  gizmo's pull requests 4
// and 5 (an editor's, labeled "meta", adding files which aren't specs; see
// tests/specpr/setup) are only imported here, together, and 6 by no test.

function doc() {
  return frame.contentDocument;
}

// The summaries (as test_SpecPullRequestSummaries leaves them).
async function synced() {
  await TestUtils.loadPath("/specpr/spec-prs");
  if (!doc().querySelector(".spec-prs-repos")) {
    // (Unless the web server's start is syncing them already, when the page
    // reloads once it's done; see tools/src/spec_prs.rs's `start`.)
    doc().querySelector(".spec-prs-sync-button")?.click();
    await waitForCondition(() => doc()?.querySelector(".spec-prs-repos, .spec-prs-error"),
                           "Syncing finishes", 500, 240);
  }
}

add_task(async function test_SpecPullRequestImports() {
  await synced();
  await TestUtils.loadPath("/specpr/spec-prs/gizmo");
  ok(doc().querySelector(".spec-prs-imports-status")?.textContent.includes("of its 8 pull requests"),
     "A spec repository's page says how many of its pull requests are imported");
  const pr = number => doc()?.querySelector(`.spec-pr[data-path="${number}"]`);
  const state = number => pr(number)?.querySelector(".spec-pr-import")?.dataset.state;
  is(pr(4).dataset.needed, "true", "and which need importing");

  const value = [...doc().querySelectorAll('.facet[data-facet="label"] .facet-value')]
    .find(button => button.textContent.includes("meta"));
  value.click();
  const button = doc().querySelector(".spec-prs-import-button");
  // (Once the facets' filtering updates it.)
  await waitForCondition(() => button.textContent == "Import these 2 pull requests",
                         "Its button imports those the facets show");
  button.click();
  await waitForCondition(() => state(4) == "current" && state(5) == "current",
                         "Importing them finishes", 500, 240);
  is(pr(4).querySelector(".spec-pr-import").textContent, "Imported", "Each says it's imported");
  is(pr(4).dataset.needed, "false", "and doesn't need importing");
  is((await (await fetch("/specpr/try-status/gizmo/6", { cache: "no-store" })).json()).state,
     "notImported", "The pull requests it didn't show weren't imported");

  // (And through the data API, which the web-server serves.)
  const imports = await (await fetch("/specpr/api/v1/spec-prs-imports?spec=gizmo", { cache: "no-store" })).json();
  ok(imports.find(each => each.number == 5)?.current, "The data API has their imports");
  const imported = await (await fetch("/specpr/api/v1/spec-prs-imported", { cache: "no-store" })).json();
  ok(imported.includes("gizmo/4") && imported.includes("gizmo/5") && !imported.includes("gizmo/6"),
     "and the pull requests the tree imported, which the indexer carries over to the next index");
  const asked = await (await fetch("/specpr/api/v1/spec-prs-import?spec=gizmo&numbers=4,5", { method: "POST" })).json();
  is(asked, 0, "and asks for those which need importing, which they don't anymore");
});
