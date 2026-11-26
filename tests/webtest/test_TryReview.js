"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 240 (it imports pushes, with their semantic analysis)

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

// (A part's strip: a source card's own, after its heading, or for a part
// shown by its mark's button, ex: a patch, the strip the button opens.)
function strip(part) {
  const own = doc().querySelector(`.review-strip[data-part="${part}"]`);
  if (own) {
    return own;
  }
  doc().querySelector(`.review-state[data-part="${part}"] .review-state-button`)?.click();
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
  // (Its lines are source listing rows: the rebased commits' code, with
  // the symbols of the listings of the indexed revision (now) and of the
  // push's rebased head (patched), and blame strips by their revisions'
  // token-centric blame.)
  const win = frame.contentWindow;
  const header = definitions.querySelector('.tr-file[data-path="widget.h"]');
  ok(header.querySelector(".now-code .source-line-with-number .blame-strip[data-hb-ctx]") &&
     header.querySelector(".then-code .source-line-with-number .blame-strip[data-hb-ctx]") &&
     Array.isArray(win.BLAME_INFOS),
     "Its lines have blame strips then and now, by their revisions' blame");
  ok(header.querySelector('.now-code[data-sym-info="patched"] [data-symbols]') &&
     header.querySelector('.then-code[data-sym-info="now"] [data-symbols]') &&
     Object.keys(win.SYM_INFOS?.patched ?? {}).length > 0 &&
     Object.keys(win.SYM_INFOS?.now ?? {}).length > 0,
     "and their symbols, now (the indexed revision's) and patched (the push's)");

  const modes = [...doc().querySelectorAll(".tr-head .then-now-modes button")].map(b => b.textContent);
  is(modes.join(", "), "Now, Patched, Now & Patched",
     "and the choice of the code now (before each patch), patched, or both");

  // (Source cards have their strips after their headings; a definition's
  // only hunk is folded onto its heading.)
  const fileHead = definitions.querySelector('.tr-file[data-path="widget.h"] .tr-file-head');
  ok(fileHead.lastElementChild?.classList.contains("review-strip"),
     "A file's card has its strip after its heading");

  // (A file's heading sticks under its section's, which sticks under its
  // patch's; see try-review.js's `measureHeads`.)
  const sectionHead = definitions.querySelector(".tr-section-head");
  const patchHead = patches[0].querySelector(".tr-patch-head");
  const top = element => parseFloat(win.getComputedStyle(element).top);
  is(win.getComputedStyle(fileHead).position, "sticky", "A file's heading sticks");
  ok(Math.abs(top(fileHead) - (top(sectionHead) + sectionHead.offsetHeight)) < 1 &&
     Math.abs(top(sectionHead) - (top(patchHead) + patchHead.offsetHeight)) < 1,
     `under its section's, which sticks under its patch's (${top(patchHead)}, ${top(sectionHead)}, ${top(fileHead)})`);

  // (Changed words' marks are underlined, a pixel thick, but not on a line
  // all of whose words are marked; see try_review.rs's `mark_word_changes`.)
  const mark = doc().querySelector(".tr-line:not(.tr-all-marked) mark.tr-word-added");
  const markStyle = mark && win.getComputedStyle(mark);
  ok(markStyle?.textDecorationLine == "underline" && markStyle.textDecorationThickness == "1px",
     `Changed words are underlined (${markStyle?.textDecorationLine} ${markStyle?.textDecorationThickness})`);
  const whole = doc().querySelector(".tr-line.tr-all-marked mark");
  if (whole) {
    is(win.getComputedStyle(whole).textDecorationLine, "none", "but not on a line all of whose words are");
  }
  is(win.getComputedStyle(doc().querySelector(".try-review .now-code")).borderTopStyle, "none",
     "(The code has no border)");
  ok(doc().querySelector(".tr-def > .tr-hunk-folded") &&
     doc().querySelector(".tr-def-head > .tr-hunk-lines"),
     "and a definition with one hunk has its lines on its heading");

  // (A patch's heading has a button of its mark, grey while it's unseen.)
  const state = patches[0].querySelector(".tr-patch-head .review-state-button");
  const icon = state?.querySelector('.review-icon[data-mark="unseen"]');
  ok(icon, "A patch's heading has a button of its mark, unseen at first");
  ok(frame.contentWindow.getComputedStyle(icon).filter.includes("grayscale"), "in grey");

  // (The review's marks.)
  const patch = patches[0].dataset.part;
  const file = definitions.querySelector('.tr-file[data-path="widget.h"]').dataset.part;
  is(markButton(file, "unseen").getAttribute("aria-pressed"), "true",
     "A file not looked at yet has its question mark pressed");
  TestUtils.click(markButton(patch, "good"));
  await waitForCondition(() => markButton(file, "good").getAttribute("aria-pressed") == "true",
                         "Marking the patch good marks its files good");
  ok(markButton(file, "good").classList.contains("review-mark-inherited"), "as inherited");
  // (A hunk with a card of its own, not folded onto its definition's.)
  const hunk = (definitions.querySelector('.tr-file[data-path="widget.h"] .tr-hunk:not(.tr-hunk-folded)') ??
                doc().querySelector(".tr-hunk:not(.tr-hunk-folded)")).dataset.part;
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

  const heights = links.map(link => link.getBoundingClientRect().height);
  ok(Math.max(...heights) < Math.min(...heights) * 1.5, "Its items don't wrap");

  TestUtils.click(wobble);
  await waitForCondition(() => doc().querySelector(".tr-def.tr-flash"), "Picking one goes to it");
  isnot(menu.style.display, "flex", "and closes the picker");
});

// (Going to parts, and marking one as being looked at, are history entries,
// whose URLs name the parts.)
add_task(async function test_TryReviewHistory() {
  await removeReview();
  await importPush();
  await loadReview();
  const win = frame.contentWindow;
  const file = doc().querySelector('.tr-file[data-path="widget.h"]');
  ok(file.id, "A file's card has an id");
  const length = win.history.length;
  TestUtils.click(markButton(file.dataset.part, "looking"));
  await waitForCondition(() => win.location.hash == `#${file.id}`,
                         "Marking a part as being looked at puts it in the URL");
  is(win.history.length, length + 1, "as a history entry");

  const definitions = doc().querySelector('.tr-section[data-section="definitions"]');
  TestUtils.click(definitions.querySelector(".tr-jump-picker"));
  const menu = doc().querySelector("#tr-jump-menu");
  await waitForCondition(() => menu.style.display == "flex", "The jump picker opens");
  const wobble = [...menu.querySelectorAll("a")].find(link => link.textContent.includes("Wobble"));
  TestUtils.click(wobble);
  await waitForCondition(() => win.location.hash && win.location.hash != `#${file.id}`,
                         "Picking a part goes to it, and puts it in the URL");
  is(win.history.length, length + 2, "as another entry");
  win.history.back();
  await waitForCondition(() => win.location.hash == `#${file.id}`,
                         "Back returns to the part looked at");
});

// (Its patch's changes' dataflow, by the push's slicing: Widget::Wobble,
// which it adds, and wobbler.cpp's call of it.)
add_task(async function test_TryReviewSlicing() {
  await removeReview();
  await importPush();
  await loadReview();
  const figure = doc().querySelector(".tr-slicing");
  ok(figure, "The patch's group has a diagram of its changes' dataflow");
  await waitForCondition(() => figure.querySelector(".tr-slice-node"), "of its changes' units");
  const nodes = [...figure.querySelectorAll(".tr-slice-node")];
  const wobble = nodes.find(node => node.textContent.includes("Wobble"));
  ok(wobble, "such as the method it adds");
  ok(figure.querySelector(".crash-graph-edge"), "linked by what they share");
  ok(wobble.querySelector('.tr-slice-mark .review-icon[data-mark="unseen"]'),
     "each with its part's mark");

  // (The marks are the review's.)
  const patch = doc().querySelector(".tr-patch").dataset.part;
  TestUtils.click(markButton(patch, "good"));
  await waitForCondition(() => wobble.querySelector(".tr-slice-mark").textContent.includes("✅"),
                         "which change with the review's");
  // (The node's handler keeps the click from following its link to its
  // source.)
  const page = doc();
  TestUtils.click(wobble);
  await waitForCondition(() => doc().querySelector(".tr-def.tr-flash, .tr-file.tr-flash"),
                         "Clicking one goes to its part");
  is(doc(), page, "on the page, rather than to its source");

  // (And a block diagram of the classes its patches change, by roll-ups.)
  const block = doc().querySelector(".tr-block svg");
  ok([...(block?.querySelectorAll("text") ?? [])].some(text => text.textContent.includes("Widget")),
     "The group has a block diagram of the classes it changes");

  ok(!figure.querySelector(".tr-slice-code"), "The diagram leaves out the code at first");
  TestUtils.click(figure.querySelector(".tr-slicing-code"));
  await waitForCondition(() => figure.querySelector(".tr-slice-code .tr-added"),
                         "and shows it when asked to");
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

// (A row's blame strip's popup, with the review's notes about its line at
// its top, if any; see try-review.js's `addLineNotes`.)
async function blamePopupNotes(row) {
  const win = frame.contentWindow;
  const strip = row.querySelector(".blame-strip");
  win.BlamePopup.triggerElement = null;
  TestUtils.dispatchMouseEvent("mouseenter", strip);
  const popup = doc().querySelector("#blame-popup");
  await waitForCondition(() => popup.style.display != "none" && win.BlamePopup.popupOwner == strip &&
                         popup.querySelector(".hb-popup"),
                         "Its blame strip's popup is shown");
  return popup.querySelector(".hb-popup > .hb-line-notes");
}

// (A later patch's change of a line: 3579ace13579's first patch adds
// stacked.cpp's Stacked, returning 1, and its second makes it return 2.)
add_task(async function test_TryReviewLaterChanges() {
  await TestUtils.loadPath("/trydelta/try/hg/3579ace13579");
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"),
                           "The push is imported", 500, 240);
  }
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "The review is loaded");
  const first = doc().querySelector('#tr-patch-1 .tr-file[data-path="stacked.cpp"]');
  const row = [...(first?.querySelectorAll(".now-code .tr-line.tr-later") ?? [])]
    .find(each => each.textContent.includes("return 1"));
  ok(row, "A line a later patch changes is marked");
  row.querySelector(".tr-text").dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  ok(!doc().querySelector(".tr-later-popup"), "Hovering over it shows no popup of its own");
  const notes = await blamePopupNotes(row);
  ok(notes?.textContent.includes("Patch 2") && notes.textContent.includes("return 2"),
     "but its blame strip's popup says which patch changes it, and the line it makes of it, at its top");
  ok(notes.compareDocumentPosition(notes.parentNode.querySelector(".hb-upper")) & Node.DOCUMENT_POSITION_FOLLOWING,
     "above its commits");

  const win = frame.contentWindow;
  const length = win.history.length;
  TestUtils.click(notes.querySelector(".tr-line-note-code"));
  await waitForCondition(() => win.location.hash.length > 1, "Clicking it goes to that patch's hunk");
  ok(doc().querySelector("#tr-patch-2")?.contains(doc().getElementById(win.location.hash.slice(1))),
     "in the later patch");
  is(win.history.length, length + 1, "as a history entry");
  is(doc().querySelector("#blame-popup").style.display, "none", "and closes the popup");

  // (Below the line's hunk, the later patch's change again, linked.)
  const group = row.closest(".tr-hunk").querySelector(".tr-later-group");
  ok(group?.querySelector(".tr-later-head")?.textContent.startsWith("Changed in patch 2"),
     `Its hunk has the later change below it (${group?.textContent})`);
  ok([...group.querySelectorAll(".now-code .tr-line.tr-added")].some(each => each.textContent.includes("return 2")) &&
     [...group.querySelectorAll(".then-code .tr-line.tr-removed")].some(each => each.textContent.includes("return 1")),
     "as that patch changes it");
  ok(group.checkVisibility(), "shown in the view's mode");
  const link = group.querySelector(".tr-later-head a");
  ok(doc().querySelector("#tr-patch-2")?.contains(doc().getElementById(link.getAttribute("href").slice(1))),
     "linked to that patch's hunk");
  const again = [...group.querySelectorAll(".now-code .tr-line")].find(each => each.textContent.includes("return 2"));
  ok(/^1:n:stacked\.cpp:\d+$/.test(again?.dataset.line ?? ""), `whose rows are that patch's lines (${again?.dataset.line})`);
});

// (A line's rows, by their keys: hovering over one highlights them; see
// try-review.js's `addLineHover`.)
add_task(async function test_TryReviewLineHover() {
  await importPush();
  await loadReview();
  const row = doc().querySelector('.tr-file[data-path="widget.h"] .now-code .tr-line.tr-added[data-line]');
  ok(row, "An added line's row has its key");
  ok(/^0:n:widget\.h:\d+$/.test(row.dataset.line), `of its patch, side, file and line (${row.dataset.line})`);
  const then = doc().querySelector('.tr-file[data-path="widget.h"] .then-code .tr-line.tr-removed[data-line]');
  ok(/^0:o:widget\.h:\d+$/.test(then?.dataset.line ?? ""), "and a removed line's, of the file as it was");
  row.querySelector(".tr-text").dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  ok(row.classList.contains("tr-hover"), "Hovering over a line highlights it");
  const others = [...doc().querySelectorAll(".tr-line.tr-hover")];
  ok(others.every(other => other.dataset.line == row.dataset.line), "and only its rows");
  doc().querySelector("#try-review").dispatchEvent(new MouseEvent("mouseleave"));
  ok(!doc().querySelector(".tr-line.tr-hover"), "until it's left");
});

// (Moved lines: f00dfeedbeef's patch moves UseWidget's code (user.cpp) and
// Unpoke's step (unpoke.cpp) to frobbed.cpp's FrobbedSize and Idle, changing
// a few of their words; see the tools' `add_moved_lines`.)
add_task(async function test_TryReviewMovedLines() {
  await TestUtils.loadPath("/trydelta/try/hg/f00dfeedbeef");
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"),
                           "The push is imported", 500, 240);
  }
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "The review is loaded");
  const file = doc().querySelector('.tr-file[data-path="frobbed.cpp"]');
  const rows = [...(file?.querySelectorAll(".now-code .tr-line") ?? [])];
  const row = text => rows.find(each => each.textContent.includes(text));
  ok(row("FrobTwice(widget)")?.classList.contains("tr-moved"),
     "A line moved from another file is marked");
  ok(row("Widget widget(aSize)")?.classList.contains("tr-moved"),
     "as is one changed a little on the way");
  ok(!row("int FrobbedSize(")?.classList.contains("tr-moved"), "but not a new one");
  // (Unpoke's step's comment, whose tokens the history follows to Idle; but
  // not its call's, whose `aWidget` stays in Unpoke's new call of Idle, and
  // whose rest is too common to follow alone; see the history's
  // `cross_file_min_len`.)
  ok(row("Set widget's state")?.classList.contains("tr-moved"),
     "A line moved from somewhere else is marked too");
  const edited = [...doc().querySelectorAll('.tr-file[data-path="user.cpp"] .now-code .tr-line.tr-added')]
    .find(each => each.textContent.includes("FrobbedSize(3, 2)"));
  ok(edited && !edited.classList.contains("tr-moved"),
     "but not a line edited where it was (UseWidget's new body)");

  const moved = row("FrobTwice(widget)");
  const notes = await blamePopupNotes(moved);
  ok(notes?.textContent.includes("Moved here from user.cpp") &&
     notes.textContent.includes("UseWidget") && notes.textContent.includes("FrobTwice(widget)"),
     `Its blame strip's popup says where it moved from: the file, its definition there, and its code (${notes?.textContent})`);
  const win = frame.contentWindow;
  TestUtils.click(notes.querySelector(".tr-line-note-code"));
  await waitForCondition(() => win.location.hash.length > 1, "Clicking it goes to where it moved from");
  ok(doc().getElementById(win.location.hash.slice(1))?.closest('.tr-file[data-path="user.cpp"]'),
     "user.cpp's hunk");

  // (In Now & Patched, each place's lines again, against what they were.)
  TestUtils.click(doc().querySelector('.tr-head .then-now-modes button[data-mode="both"]'));
  const groups = [...file.querySelectorAll(".tr-moved-group")];
  const heads = groups.map(group => group.querySelector(".tr-moved-head").textContent);
  ok(heads.some(head => head.includes("user.cpp")) && heads.some(head => head.includes("unpoke.cpp")),
     `Now & Patched has the moved lines again against where they moved from, each place's apart (${heads.join(" | ")})`);
  const group = groups.find(each => each.textContent.includes("FrobTwice"));
  ok(group.checkVisibility(), "shown");
  ok([...group.querySelectorAll(".then-code mark.tr-word-removed")].some(mark => mark.textContent == "3") &&
     [...group.querySelectorAll(".now-code mark.tr-word-added")].some(mark => mark.textContent == "aSize"),
     "with their changed words marked against what they were there");
  ok(group.querySelector(".then-code .tr-line.tr-removed .blame-strip[data-hb-ctx]"),
     "and their blame there");

  // (A moved line's rows, here and against what it was, are its rows, and
  // what it was, user.cpp's.)
  const again = [...group.querySelectorAll(".now-code .tr-line")]
    .find(each => each.textContent.includes("FrobTwice(widget)"));
  is(again?.dataset.line, moved.dataset.line, "A moved line's rows have its key");
  again.querySelector(".tr-text").dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  ok(moved.classList.contains("tr-hover"), "Hovering over one highlights the other");
  const was = [...group.querySelectorAll(".then-code .tr-line")]
    .find(each => each.textContent.includes("FrobTwice(widget)"));
  ok(/^0:o:user\.cpp:\d+$/.test(was?.dataset.line ?? ""), `and what it was, user.cpp's (${was?.dataset.line})`);
  was.querySelector(".tr-text").dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  ok(doc().querySelector('.tr-file[data-path="user.cpp"] .then-code .tr-line.tr-hover'),
     "whose row in user.cpp's hunk is highlighted with it");

  TestUtils.click(doc().querySelector('.tr-head .then-now-modes button[data-mode="now"]'));
  ok(!group.checkVisibility(), "Otherwise, they aren't shown again");
});

// (A hunk's lines around it, which its page has, shown when asked: in
// f00dfeedbeef's user.cpp, UseWidget's new line's hunk shows only its
// definition's line, but the file has two more lines after it; see the
// tools' `expander_html`.)
add_task(async function test_TryReviewExpand() {
  await TestUtils.loadPath("/trydelta/try/hg/f00dfeedbeef");
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"),
                           "The push is imported", 500, 240);
  }
  await waitForCondition(() => doc().querySelector("#try-review[data-review-ready]"),
                         "The review is loaded");
  const hunk = [...doc().querySelectorAll('.tr-file[data-path="user.cpp"] .tr-hunk')]
    .find(each => each.querySelector(".now-code")?.textContent.includes("FrobbedSize(3, 2)"));
  const now = hunk?.querySelector(".now-code");
  const then = hunk?.querySelector(".then-code");
  const button = now?.querySelector('.tr-expand[data-expand="after"] .tr-expand-button');
  ok(button?.textContent.includes("2 more lines"), "A hunk can show the file's lines after it");
  ok(!hunk.querySelector('.tr-expand[data-expand="before"]'),
     "but not before it, which another hunk shows");
  ok(!now.textContent.includes("namespace trydelta"), "which it doesn't at first");
  button.click();
  ok(now.textContent.includes("}  // namespace trydelta") && then.textContent.includes("}  // namespace trydelta"),
     "Asking shows them, on both sides");
  ok(!hunk.querySelector(".tr-expand"), "and the rows asking for them go");
  const row = side => [...side.querySelectorAll(".tr-line[data-line]")]
    .find(each => each.textContent.includes("namespace trydelta"));
  is(row(now)?.dataset.line, "0:n:user.cpp:10", "as the rows of their lines, as patched");
  is(row(then)?.dataset.line, "0:o:user.cpp:13", "and as they were");
  ok(row(now)?.querySelector(".blame-strip[data-hb-ctx]"), "with their blame");
});

// (A review's links stay its push's as new indexes rebase it again: its
// hunks' to their files by the push's own commits, which the web-server's
// `/try/.../source/` route sends to their rebased commits' files, and its
// slicing's page's by the push's name (`push=`); see the tools'
// `hunk_lines_html` and `slicing_url`.)
add_task(async function test_TryReviewDurableLinks() {
  await importPush();
  await loadReview();
  const link = doc().querySelector('.tr-file[data-path="widget.h"] .tr-hunk-lines')?.getAttribute("href");
  ok(link?.startsWith(`/trydelta/try/hg/${PUSH}/source/`) && link.includes("/widget.h#"),
     `A hunk's heading links to its file by the push's own commit (${link})`);
  const response = await fetch(link, { cache: "no-store" });
  const reached = new URL(response.url);
  ok(response.ok && /^\/trydelta\/rev\/[0-9a-f]{40}\/widget\.h$/.test(reached.pathname) &&
     reached.searchParams.get("stack") == `try/hg/${PUSH}`,
     `which goes to its rebased commit's file (${response.url})`);
  is((await fetch(`/trydelta/try/hg/${PUSH}/source/${"0".repeat(40)}/widget.h`, { cache: "no-store" })).status,
     404, "but not a commit which isn't the push's");

  const slicing = doc().querySelector(".tr-slicing a")?.getAttribute("href");
  ok(slicing?.endsWith(`&push=${PUSH}`), `The slicing's page is in the push's context by its name (${slicing})`);
  await TestUtils.loadPath(slicing);
  ok(doc().querySelector(".query-try-context"), "which the page is");
  is(doc().querySelector('#search-box input[name="push"]')?.value, PUSH,
     "and searching again stays in it, by the push's name");
  is((await fetch("/trydelta/query/default?q=Widget&push=fedcbafedcba", { cache: "no-store" })).status, 404,
     "A push which isn't imported has no context");
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
