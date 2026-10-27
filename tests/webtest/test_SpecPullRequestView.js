"use strict";

// The specpr tree's specs' rendered pages with their open pull requests'
// changes beside them (see tools/src/format.rs's `format_rendered` and
// `spec_pr_blocks_json`, and static/js/spec-pr-view.js), at
// `/specpr/rendered-prs/PATH`, from the pull requests' summaries (see
// test_SpecPullRequestSummaries.js): pull request 1 adds to the Wattsi spec's
// "Sprockets" and to the Bikeshed spec's "Gears", 2's changes conflict with the
// trunk's (its base's "turn it." is the trunk's "turn it clockwise."), and 3
// changes the Bikeshed spec's "Gears" (which only its import renders; the
// tests' order isn't known, so only these tests import it).

function doc() {
  return frame.contentDocument;
}

function key(k) {
  doc().documentElement.dispatchEvent(new frame.contentWindow.KeyboardEvent("keypress", { key: k, bubbles: true }));
}

// The summaries (as test_SpecPullRequestSummaries leaves them).
async function prepare() {
  await TestUtils.loadPath("/specpr/spec-prs");
  if (!doc().querySelector(".spec-prs-repos")) {
    doc().querySelector(".spec-prs-sync-button").click();
    await waitForCondition(() => doc()?.querySelector(".spec-prs-repos, .spec-prs-error"),
                           "Syncing finishes", 500, 240);
  }
}

async function loadView(path) {
  await TestUtils.loadPath(`/specpr/rendered-prs/${path}`);
  await waitForCondition(() => doc().body.classList.contains("sp-prs-loaded"),
                         "Its pull requests' changes load");
}

add_task(async function test_SpecPullRequestView() {
  await prepare();
  await loadView("gizmo/source");
  const entries = [...doc().querySelectorAll(".sp-pr-entry")];
  is(entries.map(entry => entry.dataset.path).join(","), "2,1",
     "The view lists the pull requests changing the spec");
  is(entries[0].dataset.state, "conflicts", "saying which conflict with the trunk");
  ok(entries[1].querySelector(".sp-pr-state").textContent.includes("1 change"),
     "and how many changes the others make");

  const cards = [...doc().querySelectorAll(".sp-pr-column .sp-pr-card")];
  is(cards.map(card => card.dataset.pr).sort().join(","), "1,2", "Its column has pull request 1's change, and 2's");
  const card = cards.find(card => card.dataset.pr == "1");
  ok(card.querySelector(".sp-pr-card-body").textContent.includes("drip oil on it"),
     "with its version of the blocks it adds");
  ok(card.querySelector(".sp-pr-card-header a").textContent.includes("#1"),
     "under its pull request's header");
  const sprocket = [...doc().querySelectorAll("#rendered-doc .rd-block")]
    .find(block => block.textContent.includes("drives a gizmo"));
  ok(Math.abs(card.getBoundingClientRect().top - sprocket.getBoundingClientRect().top) < 2,
     "beside the trunk's block it adds after");
  ok(sprocket.classList.contains("sp-pr-marked"), "which is marked");
  ok(!card.querySelector("[id]"), "without the trunk's ids");

  // Pull request 2's changes conflict with the trunk's, so its card has its
  // changes of its base's source.
  const conflict = cards.find(card => card.dataset.pr == "2");
  ok(entries[0].querySelector(".sp-pr-state").textContent.includes("1 change of its base's source"),
     "The conflicting pull request's entry says its changes of its base's source are beside the page");
  ok(conflict.querySelector(".sp-pr-card-note").textContent.includes("conflicting with the trunk's changes"),
     "whose card says they conflict");
  ok(conflict.querySelector(".sp-pr-del").textContent.endsWith("turn it.</p>"),
     "with the base's line it changes");
  is(conflict.querySelector(".sp-pr-add mark")?.textContent, " counterclockwise",
     "and its line, marking what it changes in it");
  const twist = [...doc().querySelectorAll("#rendered-doc .rd-block")]
    .find(block => block.textContent.includes("turn it clockwise"));
  ok(Math.abs(conflict.getBoundingClientRect().top - twist.getBoundingClientRect().top) < 2,
     "beside the trunk's version of the line");
  ok(twist.classList.contains("sp-pr-marked"), "which is marked too");

  // The panel goes to the changes.
  const status = () => doc().querySelector(".change-nav-status")?.textContent;
  is(status(), "2 changes", "The panel's Changes section has the cards");
  ok(!doc().querySelector(".change-nav-granularity").getClientRects().length, "without granularities");
  key("j");
  is(status(), "1 of 2 changes", "j goes to the first one");
  ok(conflict.classList.contains("change-nav-current") && twist.classList.contains("change-nav-current"),
     "marking its card and the trunk's block it changes");
  key("j");
  ok(card.classList.contains("change-nav-current") && sprocket.classList.contains("change-nav-current") &&
     !conflict.classList.contains("change-nav-current"), "j again goes to the next one");
  key("k");
  ok(conflict.classList.contains("change-nav-current"), "and k back");

  // Filtering the pull requests hides their changes.
  const value = [...doc().querySelectorAll('.facet[data-facet="review"] .facet-value')]
    .find(button => button.textContent.includes("Changes requested"));
  value.click();
  await waitForCondition(() => card.hidden, "Filtering hides the other pull request's change");
  doc().querySelector(".facet-clear").click();
  await waitForCondition(() => !card.hidden, "and showing it again shows it");

  ok(doc().querySelector('#panel-rendered-prs[href="/specpr/rendered/gizmo/source"]'),
     "The panel goes to the view without them");
  await TestUtils.loadPath("/specpr/rendered/gizmo/source");
  ok(doc().querySelector('#panel-rendered-prs[href="/specpr/rendered-prs/gizmo/source"]'),
     "and from it to the view with them");
});

add_task(async function test_SpecPullRequestViewBikeshed() {
  await prepare();
  await loadView("gizmo/bs/index.bs");
  const entry = () => doc().querySelector('.sp-pr-entry[data-path="3"]');
  const card = () => doc().querySelector('.sp-pr-column .sp-pr-card[data-pr="3"]');
  if (entry()?.dataset.state == "needs-import") {
    ok(entry().querySelector(".sp-pr-state").textContent.includes("Import it"),
       "A Bikeshed spec's view says a pull request's changes need its import to be rendered");
    ok(card()?.classList.contains("sp-pr-source-card"), "and has its changes of the source until then");
    is(card().querySelector(".sp-pr-add mark")?.textContent, ", meshing with other gears",
       "marking what it changes in the line");
    ok(card().querySelector(".sp-pr-card-note").textContent.endsWith("in its source:"),
       "saying they're the source's");
    await TestUtils.loadPath("/specpr/try/gizmo/3");
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page, .try-import-error"),
                           "Importing it finishes", 500, 240);
    is(doc().querySelector(".try-import-error")?.textContent, undefined, "without failing");
    await loadView("gizmo/bs/index.bs");
  }
  ok(card()?.querySelector(".sp-pr-card-body").textContent.includes("meshing with other gears") &&
     !card().classList.contains("sp-pr-source-card"),
     "Once it's imported, its change is there, from its import's rendering");
  ok(card().querySelector(".sp-pr-card-note").textContent.startsWith("Changes"),
     "saying it changes the paragraph");
});
