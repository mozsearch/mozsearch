"use strict";

// The lock-held: view (see tools/src/lock_held.rs) of the tests tree's
// locks/locks.cpp worker's mMutex: its guards' regions (Dispatch's, Notify's,
// which an unlock releases it in, and whose lambda it calls, Close's, after
// its monitor, and Report's, of a reporter's reference to it), the function
// Dispatch passes its guard's proof to (DispatchLockHeld), the method
// requiring it held (Status, by its annotation), but not the static mutex's
// (Increment's); the fields accessed with it held, guarded by it or not;
// its guarded field read without it (in UnsafePeek); the diagrams of how
// they're called; and the navigation panel's facets filtering them.
const MUTEX = "F_<T_mozilla::dom::Worker>_mMutex";

function doc() {
  return frame.contentDocument;
}

add_task(async function test_LockHeld() {
  await TestUtils.loadQuery("tests", `lock-held:${MUTEX}`);
  const cards = [...doc().querySelectorAll(".lock-held-card")];
  const card = name => cards.find(each => each.querySelector(".lock-held-func").textContent.endsWith(name));
  ok(card("Worker::Dispatch")?.classList.contains("lock-held-guard"), "A guard's region is a card");
  ok(card("Worker::Notify") && card("Worker::Close"), "each guard's");
  ok(card("Reporter::Report")?.classList.contains("lock-held-guard"),
     "a guard's of a field initialized from it, which is it too");
  ok(card("Worker::Status")?.classList.contains("lock-held-contract"),
     "The method requiring it held is a card too");
  ok(card("Worker::Dispatch").querySelector(".lock-held-guard-line"), "marking the guard's line");
  const proof = card("Worker::DispatchLockHeld");
  ok(proof?.classList.contains("lock-held-proof"), "The function a guard's proof is passed to is a card too");
  ok(proof.querySelector(".lock-held-how").textContent.includes("is passed its proof, from Dispatch"),
     "saying where from");
  ok(!card("Increment"), "but not another mutex's regions");
  ok(card("Worker::Notify").querySelector(".lock-held-released"), "An unlock's lines are dimmed");
  ok(card("Worker::Notify").querySelector(".lock-held-how").textContent.includes("releases it at lines"),
     "and its card says so");

  const notice = doc().querySelector(".lock-held-notice").textContent;
  ok(notice.includes("4 guards' regions in 4 functions, 1 function its guards' proofs are passed to"),
     `The notice says how many regions hold it how: ${notice}`);
  ok(notice.includes("1 method of its class requires it held (MOZ_REQUIRES); it guards 3 fields (MOZ_GUARDED_BY)"),
     "and what its class's annotations say");
  ok(notice.includes("It's also Reporter::mMutex, initialized from it"), "and which fields are it too");
  const fields = Object.fromEntries([...doc().querySelectorAll(".lock-held-fields tbody tr")]
    .map(row => [row.cells[0].textContent, [...row.cells].slice(1).map(cell => cell.textContent).join(",")]));
  is(fields["Worker::mStatus"], "4,3,1,,✓",
     "The fields accessed with it held: in how many regions, read, written, methods called, and guarded by it");
  is(fields["Worker::mQueue"], "1,,,1,✓", "methods called on a field");
  is(fields["Worker::mUnguarded"], "1,,1,,", "in a lambda called with it held, not guarded by it");
  is(fields["Reporter::mReports"], "1,1,1,,", "and with it held as the field which is it too");
  const unguarded = [...doc().querySelectorAll(".lock-held-unguarded li")].map(item => item.textContent);
  is(unguarded.join("|"), "mStatus in mozilla::dom::Worker::UnsafePeek",
     "Its guarded field's access without it held is listed (not the method requiring it held's)");
  ok(!fields["Worker::mMutex"], "but not the mutex itself");

  // The diagrams of how they're called: Dispatch's region calling the
  // function it passes its proof to, and Notify's with its unlock (Close's,
  // which makes no calls, is only a card).
  await waitForCondition(() => doc().querySelectorAll(".lock-held-graph").length, "The diagrams are drawn");
  const graphs = [...doc().querySelectorAll(".lock-held-graph")];
  is(graphs.length, 2, "A diagram for each connected part of more than one node");
  const node = text => [...doc().querySelectorAll(".lock-held-node")]
    .find(each => each.querySelector(".crash-graph-node-text").textContent.startsWith(text));
  ok(node("DispatchLockHeld")?.classList.contains("lock-held-node-proof"), "with the function a proof is passed to");
  ok(node("releases it")?.classList.contains("lock-held-node-release"), "and an unlock");
  ok(node("Dispatch ")?.closest(".lock-held-graph") == node("DispatchLockHeld").closest(".lock-held-graph"),
     "the caller with what it calls");

  // The panel's facets filter the cards.
  ok(doc().querySelector("#panel-content .facet-box"), "The facets are in the navigation panel");
  const value = (facet, path) => doc().querySelector(`.facet[data-facet="${facet}"] .facet-value[data-value="${path}"]`);
  const shown = () => cards.filter(each => !each.hidden).map(each => each.querySelector(".lock-held-func").textContent);
  value("field", "mozilla-dom-worker/mstatus/written").click();
  is(shown().join(","), "mozilla::dom::Worker::DispatchLockHeld", "Filtering by a field written with it held");
  doc().querySelector(".facet-clear").click();
  value("how", "proof").click();
  is(shown().join(","), "mozilla::dom::Worker::DispatchLockHeld", "and by how it's held");
  ok(node("releases it").closest(".lock-held-graph").hidden, "which hides the diagrams without cards shown");
  ok(node("Dispatch ").classList.contains("crash-graph-node-none") &&
     !node("DispatchLockHeld").classList.contains("crash-graph-node-none"),
     "and dims the nodes without");
  doc().querySelector(".facet-clear").click();
  is(shown().length, cards.length, "Clearing them shows them all");
});

// Search results' "Lock held" context (see tools/src/dataflow/locks.rs's
// `FileLocks`, which dataflow-summarize writes, and pipeline-server.rs's
// `results_file_facets`): the results' lines say which mutexes are held
// there, outer ones first (Close's, after its monitor), a proof's by its
// parameter's, and the navigation panel's "Lock held" facet filters them.
add_task(async function test_LockHeldContext() {
  await TestUtils.loadQuery("tests", "path:locks/locks.cpp mStatus");
  const held = [...doc().querySelectorAll(".result-locks")].map(span => span.textContent);
  ok(held.includes("holding Worker::mMutex"), `A result's line says the mutex held there: ${held}`);
  ok(held.includes("holding Worker::mMonitor › Worker::mMutex"), "outer ones first");
  ok(held.includes("holding aProofOfLock's mutex"), "and a proof's, by its parameter");
  const facet = doc().querySelector('.facet[data-facet="lock"]');
  ok(facet, "The navigation panel has a Lock held facet");
  const value = facet.querySelector('.facet-value[data-value="Worker::mMonitor"]');
  ok(value, "with the mutexes held, outer ones first");
  value.click();
  const shown = [...doc().querySelectorAll(".result-locks")].filter(span => span.getClientRects().length)
    .map(span => span.textContent);
  is(shown.join(","), "holding Worker::mMonitor › Worker::mMutex", "which filters the results");
});

// A mutex's context menu, under its diagrams, goes to where it's held, if
// the dataflow analysis found its guards' regions (its metadata's
// `lockRegions`, which crossref counts from dataflow-summarize's
// lock-regions; see crossref.rs's `add_lock_regions`), but another field's
// doesn't.
add_task(async function test_LockHeldMenu() {
  await TestUtils.resetFeatureGate("semanticInfo");
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadPath("/tests/source/locks/locks.cpp");
  const menu = doc().querySelector("#context-menu");
  const item = () => [...menu.querySelectorAll(".contextmenu-row")]
    .find(row => row.textContent.startsWith("Where "));
  TestUtils.click(doc().querySelector(`span[data-symbols="${MUTEX}"]`));
  await waitForShown(menu, "The mutex field's click shows the context menu");
  is(item()?.textContent, "Where mozilla::dom::Worker::mMutex is held (3 guards)",
     "which goes to where it's held, saying how many guards hold it");
  is(item().querySelector(".contextmenu-link").getAttribute("href"),
     `/tests/query/default?q=${encodeURIComponent(`lock-held:${MUTEX}`)}`, "the lock-held: view");
  TestUtils.click(doc().body);
  TestUtils.click(doc().querySelector('span[data-symbols="F_<T_mozilla::dom::Worker>_mStatus"]'));
  await waitForShown(menu, "Another field's click shows the context menu");
  ok(!item(), "without it");
  TestUtils.click(doc().body);
  TestUtils.click(doc().querySelector('span[data-symbols="F_<T_mozilla::dom::Cache>_mCounters"]'));
  await waitForShown(menu, "A DataMutex field's click shows the context menu");
  is(item()?.textContent, "Where mozilla::dom::Cache::mCounters is held (1 guard)", "with it too");
});

// A DataMutex's guard, which its Lock() returns (`auto counters =
// mCounters.Lock();`), holds it, and what's reached through the guard is
// accessed with it held (see tools/src/dataflow/frontend.rs's
// `cpp_lock_guard`).
add_task(async function test_LockHeldDataMutex() {
  await TestUtils.loadQuery("tests", "lock-held:F_<T_mozilla::dom::Cache>_mCounters");
  const cards = [...doc().querySelectorAll(".lock-held-card")];
  is(cards.map(card => card.querySelector(".lock-held-func").textContent).join(","), "mozilla::dom::Cache::Hit",
     "A DataMutex's guard's region is a card");
  ok(cards[0].querySelector(".lock-held-how").textContent.startsWith("holds it by AutoLock"), "by its AutoLock");
  const fields = [...doc().querySelectorAll(".lock-held-fields tbody tr")].map(row => row.cells[0].textContent);
  is(fields.join(","), "Counters::mHits", "the data reached through its guard accessed with it held");
});
