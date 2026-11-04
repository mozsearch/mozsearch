"use strict";

// The lock-held: view (see tools/src/lock_held.rs) of the tests tree's
// locks/locks.cpp worker's mMutex: its guards' regions (Dispatch's, Notify's,
// which an unlock releases it in, and whose lambda it calls, and Close's,
// after its monitor), the function Dispatch passes its guard's proof to
// (DispatchLockHeld), but not the static mutex's (Increment's); the fields
// accessed with it held; the diagrams of how they're called; and the
// navigation panel's facets filtering them.
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
  ok(notice.includes("3 guards' regions in 3 functions, 1 function its guards' proofs are passed to"),
     `The notice says how many regions hold it how: ${notice}`);
  const fields = Object.fromEntries([...doc().querySelectorAll(".lock-held-fields tbody tr")]
    .map(row => [row.cells[0].textContent, [...row.cells].slice(1).map(cell => cell.textContent).join(",")]));
  is(fields["Worker::mStatus"], "3,2,1,", "The fields accessed with it held: in how many regions, read, written, methods called");
  is(fields["Worker::mQueue"], "1,,,1", "methods called on a field");
  is(fields["Worker::mUnguarded"], "1,,1,", "and in a lambda called with it held");
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
