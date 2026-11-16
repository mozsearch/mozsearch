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
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
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

async function loadReview() {
  await TestUtils.loadQuery("trydelta", `try-review:${PUSH}`);
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
  const link = doc().querySelector(".explore-review-link");
  ok(link, "The push's page links to its review");
  const loaded = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loaded;
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "which loads with its marks");

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
