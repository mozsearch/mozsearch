"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 120 (it imports the push, with its semantic analysis)

// A try push's review view (the `try-review:` query; see the tools'
// try_review.rs and static/js/try-review.js): the trydelta tree's push
// 0123456789ab (see test_TryDelta.js) changes Widget::Frob's signature (in
// widget.h and widget.cpp), adds Widget::Wobble and wobbler.cpp, which calls
// it, and changes unpoke.cpp's implementation of a spec's algorithm; its one
// patch's changes are in the order to read them, definitions first, and each
// part has review marks, kept in the browser.

const PUSH = "0123456789ab";

function doc() {
  return frame.contentDocument;
}

async function importPush() {
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"),
                           "The push is imported", 500, 240);
  }
}

// (The review's record, from an earlier run, removed; see review-state.js.)
async function removeReview() {
  // (Not opening the database if there's none, which would make it, without
  // its store.)
  const databases = await indexedDB.databases();
  if (!databases.some(db => db.name == "searchfox-review")) {
    return;
  }
  await new Promise(resolve => {
    const request = indexedDB.open("searchfox-review");
    request.onsuccess = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains("reviews")) {
        db.close();
        resolve();
        return;
      }
      const transaction = db.transaction("reviews", "readwrite");
      transaction.objectStore("reviews").delete(`trydelta/try/${PUSH}`);
      transaction.oncomplete = () => { db.close(); resolve(); };
      transaction.onerror = () => { db.close(); resolve(); };
    };
    request.onerror = () => resolve();
  });
}

// (The push's page, which is its review once it's imported, as the
// `try-review:` query's.)
async function loadReview() {
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "The review's marks are loaded");
}

function strip(part) {
  return doc().querySelector(`.review-strip[data-part="${part}"]`);
}

function markButton(part, state) {
  return strip(part).querySelector(`.review-mark[data-state="${state}"]`);
}

add_task(async function test_TryReview() {
  await removeReview();
  await importPush();
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "The push's page, once it's imported, is its review, with its marks");
  is(doc().querySelector("#query")?.value, `try-review:${PUSH}`, "which is its query's");

  const patches = doc().querySelectorAll(".tr-patch");
  is(patches.length, 1, "It has the push's patch");
  ok(patches[0].querySelector(".tr-patch-summary").textContent.includes("wobble"),
     "with its summary");
  ok(doc().querySelector(".breadcrumbs .tr-crumbs a")?.textContent.includes("try push"),
     "The breadcrumbs lead back to the push's page");

  // (Its sections, in the order to read them: definitions before uses.)
  const sections = [...patches[0].querySelectorAll(".tr-section")].map(s => s.dataset.section);
  ok(sections.indexOf("definitions") == 0, "Its definitions come first");
  const definitions = patches[0].querySelector('.tr-section[data-section="definitions"]');
  const definitionFiles = [...definitions.querySelectorAll(".tr-file")].map(f => f.dataset.path);
  ok(definitionFiles.includes("widget.h"), "with the header");
  ok(definitionFiles.includes("wobbler.cpp"), "and the file it adds");
  ok(definitions.querySelector('.tr-def [title*="Wobble"]'),
     "and the definition it adds, under its class");
  const wobbler = definitions.querySelector('.tr-file[data-path="wobbler.cpp"]');
  ok(wobbler.querySelector(".now-code.now-only .tr-added"),
     "A new file's hunks have only their code now");

  // (Each hunk's code, then and now.)
  const pair = definitions.querySelector('.tr-file[data-path="widget.h"] .then-now');
  ok(pair?.querySelector(".then-code .tr-removed") && pair?.querySelector(".now-code .tr-added"),
     "A changed file's hunks have their code then and now");
  ok(wobbler.querySelector('.now-code .tr-text span[class^="syn_"]') &&
     doc().querySelector('.then-code .tr-text span[class^="syn_"]'),
     "with its syntax highlighted, as the source listings' is, then and now");
  ok(doc().querySelector('.then-now-modes button[data-mode="now"]'), "and the Then/Now choice");

  // (The review's marks.)
  const patch = patches[0].dataset.part;
  const file = definitions.querySelector('.tr-file[data-path="widget.h"]').dataset.part;
  is(markButton(file, "unseen").getAttribute("aria-pressed"), "true",
     "A file not looked at yet has its question mark pressed");
  TestUtils.click(markButton(patch, "good"));
  await waitForCondition(() => markButton(file, "good").getAttribute("aria-pressed") == "true",
                         "Marking the patch good marks its files good");
  ok(markButton(file, "good").classList.contains("review-mark-inherited"), "as inherited");
  const hunk = definitions.querySelector('.tr-file[data-path="widget.h"] .tr-hunk').dataset.part;
  TestUtils.click(markButton(hunk, "bad"));
  await waitForCondition(() => markButton(patch, "bad").textContent.includes("1"),
                         "The patch's strip counts a hunk marked bad");
  await waitForCondition(
    () => doc().querySelector(".tr-status-text").textContent.includes("2 marks, kept in this browser"),
    "The review's status says its marks are kept, once they're written");

  // (They're there when the page is loaded again.)
  await loadReview();
  is(markButton(hunk, "bad").getAttribute("aria-pressed"), "true", "The marks are kept");
  is(markButton(file, "good").getAttribute("aria-pressed"), "true", "with their inheritance");

  TestUtils.click(doc().querySelector(".tr-undo"));
  await waitForCondition(() => markButton(hunk, "good").getAttribute("aria-pressed") == "true",
                         "Undo takes the latest mark back");
  TestUtils.click(doc().querySelector(".tr-undo"));
  await waitForCondition(() => markButton(file, "unseen").getAttribute("aria-pressed") == "true",
                         "and then the one before it");
});

add_task(async function test_TryReviewSummary() {
  await removeReview();
  await importPush();
  await loadReview();
  const pane = doc().querySelector("#panel-content #tr-review-pane");
  ok(pane, "The navigation panel has the review's section");
  ok(pane.querySelector(".tr-status-text").textContent.includes("kept in this browser"),
     "which says its marks are kept");
  const cell = (level, state) =>
    pane.querySelector(`.review-summary-cell[data-level="${level}"][data-state="${state}"]`);
  const files = doc().querySelectorAll(".tr-file").length;
  ok(Number(cell("file", "unseen").textContent) > 0, "Its files aren't looked at yet");

  const patch = doc().querySelector(".tr-patch").dataset.part;
  TestUtils.click(markButton(patch, "looking"));
  await waitForCondition(() => cell("patch", "looking").textContent == "1",
                         "The summary counts the patch looked at");
  ok(Number(cell("file", "looking").textContent) > 0, "and its files, by inheritance");
  is(cell("file", "unseen").textContent, "\u00b7", "none of which are left not looked at");

  TestUtils.click(cell("file", "looking"));
  const items = pane.querySelectorAll(".review-summary-list a");
  ok(items.length > 0 && items.length <= 8, "Its cell lists its files, a page at a time");
  TestUtils.click(items[0]);
  await waitForCondition(() => doc().querySelector(".tr-file.tr-flash"),
                         "and its list's items go to their files");

  TestUtils.click(doc().querySelector(".tr-undo"));
  await waitForCondition(() => Number(cell("file", "unseen").textContent) > 0,
                         "Undo puts the summary back");
  ok(files > 0, "(The view has files.)");
});

// (A section's jump picker: its patch's files, classes and functions, by
// section, with their marks.)
add_task(async function test_TryReviewJumps() {
  await removeReview();
  await importPush();
  await loadReview();
  const patch = doc().querySelector(".tr-patch");
  TestUtils.click(markButton(patch.dataset.part, "good"));
  await waitForCondition(() => markButton(patch.dataset.part, "good").getAttribute("aria-pressed") == "true",
                         "The patch is marked good");

  const definitions = patch.querySelector('.tr-section[data-section="definitions"]');
  TestUtils.click(definitions.querySelector(".tr-jump-picker"));
  const menu = doc().querySelector("#tr-jump-menu");
  await waitForCondition(() => menu.style.display == "flex" && menu.querySelector("a"),
                         "A section's jump picker opens under its heading");
  ok(definitions.querySelector(".tr-section-head").contains(menu), "(which has it)");
  const labels = [...menu.querySelectorAll(".context-menu-group-label")].map(label => label.textContent);
  ok(labels[0]?.startsWith("Definitions") && labels[0].includes("this section"),
     "It lists the patch's sections, saying which one it's in");
  const links = [...menu.querySelectorAll("a")];
  ok(links.some(link => link.textContent.includes("widget.h")), "and their files");
  const wobble = links.find(link => link.textContent.includes("Wobble"));
  ok(wobble, "and their classes and functions");
  ok(wobble.textContent.includes("✅"), "with their marks (here, inherited from the patch)");

  TestUtils.click(wobble);
  await waitForCondition(() => doc().querySelector(".tr-def.tr-flash"), "Picking one goes to it");
  isnot(menu.style.display, "flex", "and closes the picker");
});

// (1a2b3c4d5e6f's two patches, "Let widgets shrink" and "Shrink widgets
// twice", don't share a file, a class or a bug, so they're two groups.)
add_task(async function test_TryReviewPatches() {
  await TestUtils.loadPath("/trydelta/try/hg/1a2b3c4d5e6f");
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"),
                           "The push is imported", 500, 240);
  }
  await TestUtils.loadQuery("trydelta", "try-review:1a2b3c4d5e6f");
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "The review is loaded");
  is(doc().querySelectorAll(".tr-patch").length, 2, "It has both patches");
  const heads = [...doc().querySelectorAll(".tr-group-head")].map(h => h.textContent);
  is(heads.length, 2, "in two groups");
  ok(heads[0].startsWith("Group 1 of 2: patch 1"), "each saying its patches");

  const link = doc().querySelector(".breadcrumbs .tr-crumb-patch-link");
  is(link?.textContent, "Patch 1 of 2", "The breadcrumbs have the patch scrolled to");
  TestUtils.click(doc().querySelector("#tr-patch-picker"));
  await waitForCondition(() => doc().querySelector("#tr-patch-menu a[data-patch]"),
                         "Its picker lists the patches");
  const labels = [...doc().querySelectorAll("#tr-patch-menu .context-menu-group-label")]
    .map(label => label.textContent);
  ok(labels[0]?.startsWith("Group 1") && labels[1]?.startsWith("Group 2"), "by their groups");
  const second = doc().querySelector('#tr-patch-menu a[data-patch="1"]');
  ok(second.textContent.includes("Patch 2: Bug 4567"), "with their summaries");
  TestUtils.click(second);
  await waitForCondition(() => link.textContent == "Patch 2 of 2",
                         "Picking one goes to it, and the breadcrumbs say so");

  // (The view's Then / Now choice, also in the breadcrumbs.)
  const small = doc().querySelector('.breadcrumbs .tr-then-now-small button[data-mode="then"]');
  ok(small, "The breadcrumbs have a copy of the Then / Now choice");
  small.click();
  is(doc().querySelector("#try-review").dataset.thenNow, "then", "which chooses for the view");
  is(doc().querySelector('.tr-head .then-now-modes button[data-mode="then"]').getAttribute("aria-pressed"),
     "true", "as the view's own does");
});

add_task(async function test_TryReviewFacets() {
  await importPush();
  await loadReview();
  const box = doc().querySelector("#panel-content .facet-box");
  ok(box?.textContent.includes("Section"), "The navigation panel's facets have the sections");
});

add_task(async function test_TryReviewNotImported() {
  await TestUtils.loadQuery("trydelta", "try-review:fedcbafedcba");
  ok(doc().body.textContent.includes("isn't imported"),
     "A push that isn't imported says so");
});
