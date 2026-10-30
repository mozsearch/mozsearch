"use strict";

// The specpr tree's specs' rendered pages with their open pull requests'
// changes beside them (see tools/src/format.rs's `format_rendered` and
// `spec_pr_blocks_json`, and static/js/spec-pr-view.js), at
// `/specpr/rendered-prs/PATH`, from the pull requests' summaries (see
// test_SpecPullRequestSummaries.js): pull request 1 adds to the Wattsi spec's
// "Sprockets" and to the Bikeshed spec's "Gears", 2's changes conflict with the
// trunk's (its base's "turn it." is the trunk's "turn it clockwise."), 3
// changes the Bikeshed spec's "Gears" (which only its import renders; the
// tests' order isn't known, so only these tests import it), and 6 adds to the
// file the ecmarkup spec gizmo/es/spec.html imports, gauges.html.

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
    // (Unless the web server's start is syncing them already, when the page
    // reloads once it's done; see tools/src/spec_prs.rs's `start`.)
    doc().querySelector(".spec-prs-sync-button")?.click();
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
  // (7 and 8 too, whose first commits are 1's, and have no changes of their
  // own to this page; see test_SpecPullRequestViewStacked.)
  is(entries.map(entry => entry.dataset.path).join(","), "8,7,2,1",
     "The view lists the pull requests changing the spec");
  const entry = number => entries.find(each => each.dataset.path == number);
  is(entry("2").dataset.state, "conflicts", "saying which conflict with the trunk");
  ok(entry("1").querySelector(".sp-pr-state").textContent.includes("1 change"),
     "and how many changes the others make");

  const cards = [...doc().querySelectorAll(".sp-pr-column .sp-pr-card")];
  is(cards.map(card => card.dataset.pr).sort().join(","), "1,2", "Its column has pull request 1's change, and 2's");
  const card = cards.find(card => card.dataset.pr == "1");
  ok(card.querySelector(".sp-pr-card-body").textContent.includes("drip oil on it"),
     "with its version of the blocks it adds");
  ok([...frame.contentWindow.CSS.highlights.get("sp-pr-added")]
       .some(range => card.contains(range.startContainer) && range.toString().includes("drip oil on it")),
     "highlighted as added");
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
  ok(entry("2").querySelector(".sp-pr-state").textContent.includes("1 change of its base's source"),
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
  // (Clicking a card makes it the current change, which j and k go on from.)
  card.scrollIntoView({ block: "center" });
  card.querySelector(".sp-pr-card-note").click();
  ok(card.classList.contains("change-nav-current") && sprocket.classList.contains("change-nav-current") &&
     !conflict.classList.contains("change-nav-current"), "Clicking a card makes it the current change");
  is(status(), "2 of 2 changes", "as the panel says");
  key("k");
  ok(conflict.classList.contains("change-nav-current"), "and k goes on from it");
  ok(cards.every(each => each.querySelector(".sp-pr-card-header").getClientRects().length),
     "Every card has its pull request's header");

  // The facets and the list are in the navigation panel (the fancy bar, if
  // it's docked), the list's heading saying how many pull requests the
  // filtering shows.
  ok(doc().querySelector("#panel-content .facet-box") && doc().querySelector("#panel-content .sp-pr-list"),
     "The facets and the list are in the panel");
  ok(!doc().querySelector(".sp-prs-header .facet-box, .sp-prs-header .sp-pr-list"),
     "rather than before the page");
  const count = () => doc().querySelector(".sp-pr-list-count").textContent;
  is(count(), "(4)", "The list's heading says how many pull requests it has");

  // Filtering the pull requests hides their changes.
  const value = [...doc().querySelectorAll('.facet[data-facet="review"] .facet-value')]
    .find(button => button.textContent.includes("Changes requested"));
  value.click();
  await waitForCondition(() => card.hidden, "Filtering hides the other pull request's change");
  is(count(), "(1 of 4)", "and the list's heading says how many it shows");
  doc().querySelector(".facet-clear").click();
  await waitForCondition(() => !card.hidden, "and showing it again shows it");
  is(count(), "(4)", "as its heading does");

  ok(doc().querySelector('#panel-rendered-prs[href="/specpr/rendered/gizmo/source"]'),
     "The panel goes to the view without them");
  await TestUtils.loadPath("/specpr/rendered/gizmo/source");
  ok(doc().querySelector('#panel-rendered-prs[href="/specpr/rendered-prs/gizmo/source"]'),
     "and from it to the view with them");
});

// A card's "Hide" hides its pull request's cards and entry, whatever the
// facets' filtering, on the spec's pages (pull request 1 changes both the
// Wattsi spec's page and the Bikeshed spec's), until the panel's list of
// hidden pull requests shows it again (see spec-pr-view.js's `setHidden`).
add_task(async function test_SpecPullRequestViewHide() {
  await prepare();
  const storage = frame.contentWindow.localStorage;
  const hiddenKey = () => frame.contentWindow.storageKey("spPrHidden/specpr/gizmo");
  storage.removeItem(hiddenKey());
  await loadView("gizmo/source");
  const card = number => doc().querySelector(`.sp-pr-column .sp-pr-card[data-pr="${number}"]`);
  const entry = number => doc().querySelector(`.sp-pr-entry[data-path="${number}"]`);
  const count = () => doc().querySelector(".sp-pr-list-count").textContent;
  const hiddenBox = () => doc().querySelector(".sp-pr-hidden-box");
  ok(!hiddenBox(), "Without hidden pull requests, the panel doesn't list them");

  card(1).querySelector(".sp-pr-hide").click();
  ok(card(1).hidden && !card(2).hidden, "A card's Hide hides its pull request's cards");
  ok(!entry(1).getClientRects().length, "and its entry");
  is(count(), "(3 of 4)", "which the list's heading counts");
  ok(hiddenBox()?.querySelector('.sp-pr-hidden-entry[data-pr="1"]')?.textContent.startsWith("#1 "),
     "The panel lists it as hidden");
  ok(hiddenBox().compareDocumentPosition(doc().querySelector(".sp-pr-list-box")) & Node.DOCUMENT_POSITION_PRECEDING,
     "after the list of pull requests");

  // (Filtering, and clearing the filter, leave it hidden.)
  [...doc().querySelectorAll('.facet[data-facet="review"] .facet-value')]
    .find(button => button.textContent.includes("Changes requested")).click();
  await waitForCondition(() => count() == "(1 of 4)", "Filtering");
  doc().querySelector(".facet-clear").click();
  await waitForCondition(() => count() == "(3 of 4)", "Clearing the filter");
  ok(card(1).hidden && !card(2).hidden, "shows the others' cards, but not the hidden one's");

  await loadView("gizmo/source");
  ok(card(1)?.hidden && hiddenBox(), "It's hidden on the next page");
  await loadView("gizmo/bs/index.bs");
  await waitForCondition(() => card(1), "and on the spec's other pages");
  ok(card(1).hidden, "too");

  hiddenBox().querySelector('.sp-pr-hidden-entry[data-pr="1"] .sp-pr-unhide').click();
  ok(!card(1).hidden && !hiddenBox(), "Its Show in the panel shows it again");
  ok(storage.getItem(hiddenKey()) === null, "which is remembered");
  await loadView("gizmo/source");
  ok(!card(1).hidden, "on the next page");
});

// The list of pull requests in the panel collapses (a long one at first),
// which is remembered, while filtering and on the next page, until it's
// expanded again.
add_task(async function test_SpecPullRequestViewListCollapses() {
  await prepare();
  await loadView("gizmo/source");
  const storage = frame.contentWindow.localStorage;
  const storageKey = frame.contentWindow.storageKey("spPrListOpen");
  storage.removeItem(storageKey);
  await loadView("gizmo/source");
  const box = () => doc().querySelector(".sp-pr-list-box");
  const count = () => doc().querySelector(".sp-pr-list-count").textContent;
  ok(box().open, "A short list is expanded at first");
  box().querySelector("summary").click();
  ok(!box().open, "Its heading collapses it");
  await waitForCondition(() => storage.getItem(storageKey) == "false", "which is remembered");
  [...doc().querySelectorAll('.facet[data-facet="review"] .facet-value')]
    .find(button => button.textContent.includes("Changes requested")).click();
  await waitForCondition(() => count() == "(1 of 4)", "Filtering");
  ok(!box().open, "leaves it collapsed, its heading saying how many it shows");
  await loadView("gizmo/source");
  ok(!box().open, "and it's collapsed on the next page");
  box().querySelector("summary").click();
  ok(box().open, "until it's expanded");
  await waitForCondition(() => storage.getItem(storageKey) == "true", "which is remembered too");
  await loadView("gizmo/source");
  ok(box().open, "on the next page");
  storage.removeItem(storageKey);
});

// Pull requests whose first commits are another's (see the tools'
// `spec_prs::containments`): 7 is stacked on 1 (its two commits and one
// more), and 8 starts with an earlier version of 1's first commit (by its
// message: the same, by the same author, with other words).  Their entries
// say so, and 1's that they have its commits, and their cards have only their
// own changes: none of the Wattsi spec's, which only 1's commits change, and
// in the Bikeshed spec's source, their own lines but not 1's.
add_task(async function test_SpecPullRequestViewStacked() {
  await prepare();
  await loadView("gizmo/source");
  const entry = number => doc().querySelector(`.sp-pr-entry[data-path="${number}"]`);
  const note = (number, kind) => entry(number)?.querySelector(`.sp-pr-${kind}`)?.textContent;
  is(note(7, "contains"), "Its first 2 commits are #1's; only its own changes are shown here.",
     "A stacked pull request's entry says which pull request's commits it starts with");
  is(entry(7).querySelector(".sp-pr-contains a")?.getAttribute("href"), "/specpr/try/gizmo/1",
     "linking to it");
  ok(JSON.parse(entry(7).dataset.pr).blocks.includes("?after="),
     "and its changes are only those after them");
  is(note(8, "contains"),
     "Its first commit is an earlier version of #1's first (of 2), by its message; only its own changes are shown here.",
     "So does one which starts with an earlier version of its commit");
  is(note(1, "contained"),
     "Some of its commits (or versions of them) are in #7 and #8 too, which show only their own changes.",
     "and the other's entry says which have its commits");
  for (const number of [7, 8]) {
    is(entry(number).querySelector(".sp-pr-state").textContent, "No changes on this page",
       `#${number} has no changes of its own to the page 1's commits change`);
    ok(!doc().querySelector(`.sp-pr-column .sp-pr-card[data-pr="${number}"]`), "and no cards");
  }

  await loadView("gizmo/bs/index.bs");
  const card = number => doc().querySelector(`.sp-pr-column .sp-pr-card[data-pr="${number}"]`);
  await waitForCondition(() => card(7) && card(8), "Their cards for the Bikeshed spec's changes");
  ok(card(7).textContent.includes("ratchet") && !card(7).textContent.includes("cog"),
     "The stacked pull request's card has its own change of the source, without 1's");
  ok(card(8).textContent.includes("flywheel"), "and the other's, its own");
  ok(card(1).textContent.includes("cog"), "which 1's card has");
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
  ok(card().querySelector(".sp-pr-card-body").classList.contains("rendered-doc"),
     "styled as the page's blocks are");
  // (What it changes of the trunk's paragraph is highlighted; see
  // spec-pr-view.js's `diffWords`.)
  const highlighted = name => [...frame.contentWindow.CSS.highlights.get(name)]
        .filter(range => card().contains(range.startContainer))
        .map(range => range.toString());
  ok(highlighted("sp-pr-added").join(" ").includes("meshing with other gears"),
     `highlighting the words it adds (${highlighted("sp-pr-added")})`);
  ok(!highlighted("sp-pr-added").join(" ").includes("turns a gizmo"),
     "but not those it keeps (\"A gear turns a gizmo.\" is the trunk's)");
});

add_task(async function test_SpecPullRequestViewImported() {
  await prepare();
  await loadView("gizmo/es/spec.html");
  const link = doc().querySelector('.sp-pr-entry[data-path="6"] .sp-pr-imported a');
  is(link?.getAttribute("href"), "/specpr/rendered-prs/gizmo/es/gauges.html",
     "An ecmarkup spec's view lists the pull requests changing the files it imports, linking to their views");
  await loadView("gizmo/es/gauges.html");
  const card = doc().querySelector('.sp-pr-column .sp-pr-card[data-pr="6"]');
  ok(card?.querySelector(".sp-pr-card-body")?.textContent.includes("needle points at the gizmo's speed"),
     "whose views have their changes, rendered");
  const gauge = [...doc().querySelectorAll("#rendered-doc .rd-block")]
    .find(block => block.textContent.includes("how fast a gizmo turns"));
  ok(gauge && Math.abs(card.getBoundingClientRect().top - gauge.getBoundingClientRect().top) < 2,
     "beside the block they add after");
});

// Going to a definition on the page (ex: from the index of the terms the spec
// defines) doesn't reload it, which would load its pull requests' changes
// again, nor does going back (see search.js's popstate handler).
// A definition's "Anchor to" in the view (see test_RenderedSpecContextMenu)
// anchors the page's URL to it and hides the menu, which works again after,
// without reloading the page (which would load its pull requests' changes
// again), even if it's clicked twice.
add_task(async function test_SpecPullRequestViewAnchorTo() {
  await prepare();
  await loadView("gizmo/bs/index.bs");
  frame.contentWindow.notReloaded = true;
  const errors = [];
  frame.contentWindow.addEventListener("error", event => errors.push(event.message));
  const menu = doc().querySelector("#context-menu");
  const anchorItem = () => [...menu.querySelectorAll("a")].find(a => a.textContent.trim().startsWith("Anchor to"));
  TestUtils.click(doc().querySelector('dfn[id="gear"]'));
  await waitForShown(menu, "The definition's click shows the context menu");
  const anchor = anchorItem();
  is(anchor?.textContent.trim(), "Anchor to gear", "which can anchor the page's URL to it");
  anchor.click();
  is(frame.contentWindow.location.hash, "#gear", "which anchoring does");
  is(menu.style.display, "none", "hiding the menu");
  anchor.click();
  await new Promise(resolve => setTimeout(resolve, 500));
  is(frame.contentWindow.location.hash, "#gear", "and clicking it again doesn't go to its link's #");
  ok(frame.contentWindow.notReloaded && doc().body.classList.contains("sp-prs-loaded"),
     "without reloading the page");
  TestUtils.click(doc().querySelector('dfn[id="gear"]'));
  await waitForShown(menu, "The menu shows again");
  ok(anchorItem(), "with the item");
  is(errors.join("\n"), "", "without errors");
  TestUtils.click(doc().body);
});

add_task(async function test_SpecPullRequestViewSamePageDefinition() {
  await prepare();
  await loadView("gizmo/bs/index.bs");
  frame.contentWindow.notReloaded = true;
  const menu = doc().querySelector("#context-menu");
  TestUtils.click(doc().querySelector('.sp-index a[data-rendered-def="#gear"]'));
  await waitForShown(menu, "The index's link's click shows the context menu");
  const def = [...menu.querySelectorAll("a")].find(a => a.textContent.includes("Go to definition"));
  is(def?.getAttribute("href"), "#gear", "which goes to the definition on the page");
  def.click();
  await waitForCondition(() => frame.contentWindow.location.hash == "#gear", "Going to it");
  await new Promise(resolve => setTimeout(resolve, 500));
  ok(frame.contentWindow.notReloaded && doc().body.classList.contains("sp-prs-loaded"),
     "doesn't reload the page");
  frame.contentWindow.history.back();
  await waitForCondition(() => frame.contentWindow.location.hash != "#gear", "Going back");
  await new Promise(resolve => setTimeout(resolve, 500));
  ok(frame.contentWindow.notReloaded, "doesn't either");
});
