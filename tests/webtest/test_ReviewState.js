"use strict";

// The review marks' state (static/js/review-state.js): marks, their
// inheritance and counts, undo, notes, keeping them in IndexedDB, sharing
// them with other tabs over a BroadcastChannel, and the icon strips.  (The
// module runs in the harness's own window, as pages' Trusted Types don't
// let a test add scripts to them; two reviews of the same key there talk
// over their channels as two tabs' would.)

async function loadModule() {
  if (!window.ReviewState) {
    await new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/tests/static/js/review-state.js";
      script.onload = resolve;
      script.onerror = reject;
      document.head.append(script);
    });
  }
  return window.ReviewState;
}

// A review of a made-up patch: a file, a class with two methods, one of
// them with a hunk.
function addParts(review) {
  review.addPart("p:1", null, "patch", "Patch 1");
  review.addPart("f:1:a.cpp", "p:1", "file", "a.cpp");
  review.addPart("d:1:a.cpp:A", "f:1:a.cpp", "class", "A");
  review.addPart("d:1:a.cpp:A::m", "d:1:a.cpp:A", "function", "A::m");
  review.addPart("d:1:a.cpp:A::n", "d:1:a.cpp:A", "function", "A::n");
  review.addPart("h:1:a.cpp:x", "d:1:a.cpp:A::m", "hunk", "a.cpp hunk");
  return review;
}

async function openReview(key) {
  const ReviewState = await loadModule();
  return addParts(new ReviewState.Review(key, [["rev", "abc"], ["bug", 123]])).open();
}

// (Each task's own review, which it removes at the end.)
function uniqueKey(name) {
  return `tests/review-test/${name}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function withStore(mode, fn) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(ReviewState.DB_NAME);
    request.onsuccess = () => {
      const db = request.result;
      const transaction = db.transaction(ReviewState.STORE, mode);
      let result;
      Promise.resolve(fn(transaction.objectStore(ReviewState.STORE))).then(r => { result = r; });
      transaction.oncomplete = () => { db.close(); resolve(result); };
      transaction.onerror = () => { db.close(); reject(transaction.error); };
    };
    request.onerror = () => reject(request.error);
  });
}

async function removeReview(...reviews) {
  for (const review of reviews) {
    await review.close();
  }
  await withStore("readwrite", store => store.delete(reviews[0].key));
}

add_task(async function test_ReviewStateMarks() {
  const key = uniqueKey("marks");
  const review = await openReview(key);
  ok(review.persistent, "The review's marks are kept in IndexedDB");

  is(review.effective("d:1:a.cpp:A::m").state, "unseen", "Nothing is marked at first");

  await review.mark("d:1:a.cpp:A", "good");
  let mark = review.effective("d:1:a.cpp:A::m");
  is(mark.state, "good", "Marking a class good marks its methods good");
  is(mark.from, "d:1:a.cpp:A", "as its mark, inherited");
  is(review.own("d:1:a.cpp:A::m"), null, "without marking them");
  is(review.effective("h:1:a.cpp:x").from, "d:1:a.cpp:A", "and their hunks");

  await review.mark("d:1:a.cpp:A::n", "bad");
  is(review.effective("d:1:a.cpp:A::n").state, "bad", "A method marked bad stays bad");
  await review.mark("d:1:a.cpp:A", "good");
  is(review.effective("d:1:a.cpp:A::n").state, "bad", "even when its class is marked good again");
  is(review.counts("f:1:a.cpp").counts.bad, 1, "Its file counts the bad mark under it");
  is(review.counts("f:1:a.cpp").counts.good, 1, "and the good one");
  is(review.partsWith("function", "good").join(","), "d:1:a.cpp:A::m",
     "The methods marked good, by inheritance");

  await review.note("d:1:a.cpp:A::n", "Leaks the principal");
  is(review.noteOf("d:1:a.cpp:A::n"), "Leaks the principal", "A note is kept");
  is(review.counts("p:1").notes, 1, "and counted");

  // Undo walks back the changes, latest first.
  await review.undo();
  is(review.noteOf("d:1:a.cpp:A::n"), "", "Undo removes the note");
  await review.undo();
  is(review.own("d:1:a.cpp:A::n"), null, "then the bad mark");
  is(review.effective("d:1:a.cpp:A::n").state, "good", "which now inherits its class's");
  await review.undo();
  is(review.own("d:1:a.cpp:A"), null, "then the class's mark");
  ok(!review.canUndo(), "and then there's nothing left to undo");

  await review.mark("h:1:a.cpp:x", "unseen");
  await review.mark("d:1:a.cpp:A", "good");
  is(review.effective("h:1:a.cpp:x").state, "unseen",
     "A part marked as not looked at yet stays so in a part marked good");

  await removeReview(review);
});

add_task(async function test_ReviewStatePersists() {
  const key = uniqueKey("persists");
  let review = await openReview(key);
  await review.mark("f:1:a.cpp", "looking");
  await review.note("p:1", "Check the IPC changes");
  await review.close();

  // (As a page loaded again would.)
  review = await openReview(key);
  is(review.own("f:1:a.cpp"), "looking", "The marks are there after a reload");
  is(review.noteOf("p:1"), "Check the IPC changes", "and the notes");
  ok(review.canUndo(), "and the log, for undo");
  await review.undo();
  is(review.noteOf("p:1"), "", "which undoes the latest change");

  const records = await withStore("readonly", store => new Promise(resolve => {
    const get = store.index("indices").getAll(IDBKeyRange.only(["bug", 123]));
    get.onsuccess = () => resolve(get.result);
  }));
  const record = records.find(r => r.key == key);
  ok(record, "The record can be found by what it's of (its bug)");
  is(record.data.marks["f:1:a.cpp"].state, "looking", "and has the marks");

  await removeReview(review);
});

add_task(async function test_ReviewStateTabs() {
  const key = uniqueKey("tabs");
  const first = await openReview(key);
  const second = await openReview(key);

  await first.mark("d:1:a.cpp:A", "good");
  await waitForCondition(() => second.own("d:1:a.cpp:A") == "good",
                         "The other page gets the mark");
  is(second.effective("d:1:a.cpp:A::m").from, "d:1:a.cpp:A", "and its methods inherit it");

  // (Changes made in both at once end up the same in both.)
  await Promise.all([
    first.mark("d:1:a.cpp:A::m", "bad"),
    second.mark("d:1:a.cpp:A::n", "looking"),
  ]);
  await waitForCondition(
    () => first.own("d:1:a.cpp:A::n") == "looking" && second.own("d:1:a.cpp:A::m") == "bad",
    "Both pages get both changes");
  is(JSON.stringify(first.data.marks), JSON.stringify(second.data.marks),
     "and have the same marks");

  await second.undo();
  await waitForCondition(() => first.own("d:1:a.cpp:A::n") === null || first.own("d:1:a.cpp:A::m") === null,
                         "Undo in one page undoes in the other too");

  await removeReview(first, second);
});

add_task(async function test_ReviewStateStrips() {
  const key = uniqueKey("strips");
  const review = await openReview(key);
  const box = document.createElement("div");
  const classStrip = ReviewState.strip(review, "d:1:a.cpp:A");
  const methodStrip = ReviewState.strip(review, "d:1:a.cpp:A::m");
  box.append(classStrip, methodStrip);
  document.body.append(box);
  ReviewState.bind(review, box);

  const button = (strip, state) => strip.querySelector(`.review-mark[data-state="${state}"]`);
  is(button(methodStrip, "unseen").getAttribute("aria-pressed"), "true",
     "A part not looked at yet has its question mark pressed");

  TestUtils.click(button(classStrip, "good"));
  await waitForCondition(() => button(methodStrip, "good").getAttribute("aria-pressed") == "true",
                         "Marking the class good presses its method's good mark");
  ok(button(methodStrip, "good").classList.contains("review-mark-inherited"),
     "as inherited");
  ok(button(methodStrip, "good").title.includes("(from A)"), "from the class");

  TestUtils.click(button(methodStrip, "bad"));
  await waitForCondition(() => button(classStrip, "bad").textContent.includes("1"),
                         "The class's strip counts the bad mark under it");

  TestUtils.click(button(methodStrip, "bad"));
  await waitForCondition(() => review.own("d:1:a.cpp:A::m") === null,
                         "Clicking a part's own mark again clears it");

  TestUtils.click(methodStrip.querySelector(".review-note-button"));
  const area = document.querySelector(".review-note-editor textarea");
  ok(area, "The note button opens an editor");
  area.value = "Needs a test";
  area.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await waitForCondition(() => review.noteOf("d:1:a.cpp:A::m") == "Needs a test",
                         "Enter keeps the note");
  ok(methodStrip.querySelector(".review-note-button").classList.contains("review-note-present"),
     "and the note button shows there's one");

  box.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true }));
  await waitForCondition(() => review.noteOf("d:1:a.cpp:A::m") == "",
                         "Ctrl+Z undoes the note");

  box.remove();
  await removeReview(review);
});
