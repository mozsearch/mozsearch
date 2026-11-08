"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 120 (it imports a push, with its semantic analysis, and
// waits for its pages, as test_TryDelta.js's tasks do)

// The trydelta tree's try push fdb97531eca8 renames gizmo.rs's Gizmo::spin
// to twirl, but not lib.rs's calls of it.  Its Rust analysis (see
// scripts/try-analyze-rust.sh) is rust-analyzer's, as the tree's is, of the
// files whose analysis it can change (see try-rust-files): gizmo.rs, which it
// changes, and lib.rs, which uses its symbols, whose calls are then of
// nothing.

const PUSH = "fdb97531eca8";

function doc() {
  return frame.contentDocument;
}

// Import the push (if it isn't yet), and return the head of its stack rebased
// onto the indexed revision (as test_TryDelta.js's `importPush` does).
async function importPush() {
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  const link = doc().querySelector(".explore-semantic-head a")?.getAttribute("href");
  ok(link?.startsWith("/trydelta/rev/"), "The push's page links to its semantic analysis");
  return link.split("/")[3];
}

// The symbols of the line's tokens whose text is `text`.
function symbolsOn(line, text) {
  return [...doc().querySelectorAll(`#line-${line} span[data-symbols]`)]
    .filter(span => span.textContent == text)
    .flatMap(span => span.dataset.symbols.split(","));
}

add_task(async function test_TryDeltaRustAnalysis() {
  const head = await importPush();

  await TestUtils.loadPath("/trydelta/source/gizmos/gizmo.rs");
  const spin = symbolsOn(12, "spin");
  ok(spin.length, "The indexed revision's method has its symbol");
  await TestUtils.loadPath(`/trydelta/rev/${head}/gizmos/gizmo.rs`);
  const twirl = symbolsOn(12, "twirl");
  ok(twirl.length && !twirl.some(sym => spin.includes(sym)),
     "and the push's renamed method has its own");

  await TestUtils.loadPath("/trydelta/source/gizmos/lib.rs");
  ok(symbolsOn(9, "spin").some(sym => spin.includes(sym)), "The indexed revision's lib.rs calls it");
  await TestUtils.loadPath(`/trydelta/rev/${head}/gizmos/lib.rs`);
  ok(!symbolsOn(9, "spin").some(sym => spin.includes(sym)),
     "but the push's doesn't, though the push doesn't change lib.rs (it uses gizmo.rs's symbols)");

  const response = await fetch(`/trydelta/rev-canvas/${head}/canvas.json`, { cache: "no-store" });
  const canvas = await response.json();
  is(canvas.other_files.filter(file => file.note == "not analyzed").map(file => file.path).join(", "), "",
     "The push's Rust is analyzed");
  ok(canvas.nodes.some(node => node.path == "gizmos/gizmo.rs" && twirl.includes(node.id)),
     "and its renamed method is a card");
});
