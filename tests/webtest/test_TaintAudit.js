"use strict";

// Taint audits (see tools/src/dataflow/taint.rs): `taint-audit:<type>` maps
// the functions receiving a type's values from content processes (the `Recv`
// methods of messages with parameters of the type), with their parameters'
// validation statuses.

add_task(async function test_TaintAudit() {
  await TestUtils.loadQuery("tests", "taint-audit:SIntDouble");
  const doc = frame.contentDocument;
  await waitForCondition(() => doc.documentElement.dataset.subwayReady, "The audit's map is drawn");
  const model = frame.contentWindow.SUBWAY;
  const card = model.cards.find(c => c.func.endsWith("::RecvTestStruct"));
  ok(card, "The map has the Recv method receiving the type from content processes");
  const source = model.taint?.sources.find(s => s.card == card.id);
  is(source?.status, "unvalidated", "whose parameter isn't validated before its uses");
  ok(doc.querySelector(".sw-card .sw-track.sw-tainted"), "Its line comes in tainted (dashed)");
  ok(doc.querySelector(".sw-card .sw-dot-tainted"), "and stops tainted");
  ok(doc.querySelector("#sw-legend .sw-status-unvalidated"), "The legend lists it as unvalidated");
});
