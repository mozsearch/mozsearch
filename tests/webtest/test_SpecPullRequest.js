"use strict";

// The specpr tree, a tree of specs (see tests/specpr/setup), imports its spec
// repository's pull requests as try pushes (see tools/src/spec_forest.rs and
// tests/specpr/import-try), at `/specpr/try/SPEC/NUMBER`: gizmo's pull
// request 1 adds to the spec in two commits, and pull request 2 changes what
// the trunk changed since (so it can't be rebased onto the tree's revision).

function doc() {
  return frame.contentDocument;
}

async function statusOf(pr) {
  return (await fetch(`/specpr/try-status/gizmo/${pr}`, { cache: "no-store" })).json();
}

async function importPage(pr) {
  await TestUtils.loadPath(`/specpr/try/gizmo/${pr}`);
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    // (Or says why it failed.)
    await waitForCondition(() => doc()?.querySelector(".explore-page, .try-import-error"),
                           "Importing it finishes", 500, 240);
    is(doc().querySelector(".try-import-error")?.textContent, undefined, "without failing");
  }
}

add_task(async function test_SpecPullRequest() {
  await TestUtils.loadPath("/specpr/try/gizmo/1");
  is(doc().querySelector(".try-import h2")?.textContent, "Pull request #1 of gizmo",
     "A spec's pull request's page names it");
  ok(doc().querySelector(".try-import-status")?.textContent.includes("hasn't imported"),
     "and says the tree hasn't imported it");

  await importPage(1);
  is(doc().querySelector(".explore-page h2")?.textContent, "Pull request #1 of gizmo",
     "Its explore page is titled by it");
  const commits = [...doc().querySelectorAll(".explore-commit-list .explore-commit")];
  is(commits.length, 2, "with its two commits");
  ok(commits[0].textContent.includes("Define oiling sprockets") &&
     commits[1].textContent.includes("Add springs"), "oldest first");
  ok(doc().querySelector(".explore-pull-request .explore-pull-request-refresh"),
     "and a button importing its newer commits");
  is((await statusOf(1)).state, "imported", "Its status says it's imported");

  const link = doc().querySelector('.explore-file-header a[href*="gizmo/source"]');
  ok(link?.getAttribute("href").endsWith("?stack=try/gizmo/1"),
     "Its files link to their views in its context");
  await TestUtils.loadPath(link.getAttribute("href"));
  await waitForCondition(() => doc().querySelector(".stack-strip"),
                         "which show which of its commits changed each line");
  ok(doc().querySelector(".panel .change-nav"), "and can go to its changes");

  // Its head's rendered specs, at its revision: the Wattsi spec rendered by
  // the web server, and the Bikeshed spec by its import (see render-spec-pr).
  const head = link.getAttribute("href").match(/\/rev\/([0-9a-f]{40})\//)[1];
  await TestUtils.loadPath(`/specpr/rev-rendered/${head}/gizmo/source`);
  ok(doc().querySelector(".rendered-doc")?.textContent.includes("stores a gizmo's energy"),
     "Its head's rendered spec has its changes");
  // (Bikeshed makes the apostrophe of "a gear's tooth" a typographic one.)
  await TestUtils.loadPath(`/specpr/rev-rendered/${head}/gizmo/bs/index.bs`);
  ok(doc().querySelector(".rendered-doc")?.textContent.includes("tooth"),
     "and so does its head's rendered Bikeshed spec");
  await TestUtils.loadPath("/specpr/rendered/gizmo/bs/index.bs");
  const tip = doc().querySelector(".rendered-doc")?.textContent || "";
  ok(tip.includes("turns a gizmo") && !tip.includes("tooth"),
     "while the tip's is the trunk's");
});

add_task(async function test_SpecPullRequestConflicting() {
  await importPage(2);
  const commits = [...doc().querySelectorAll(".explore-commit-list .explore-commit")];
  is(commits.length, 1, "A pull request conflicting with the trunk is imported too");
  ok(commits[0].textContent.includes("Twist gizmos the other way"), "with its commit");
});

add_task(async function test_SpecPullRequestURLs() {
  for (const path of ["/specpr/try/hg/abcdef012345", "/specpr/try/nope/1", "/specpr/try/gizmo/01",
                      "/specpr/try-status/gizmo/x"]) {
    is((await fetch(path, { cache: "no-store" })).status, 404, `${path} isn't a pull request's`);
  }
});

add_task(async function test_SpecPullRequestReimport() {
  await importPage(1);
  // Asking again imports its newer commits (here, the same ones).
  const response = await fetch("/specpr/try/gizmo/1", { method: "POST" });
  ok(response.redirected && response.url.endsWith("/specpr/try/gizmo/1?refresh"),
     "Asking an imported pull request's import again goes to its page");
  let status;
  for (let i = 0; i < 240; i++) {
    status = await statusOf(1);
    if (status.state == "imported") {
      break;
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  is(status.state, "imported", "which imports it again");
});
