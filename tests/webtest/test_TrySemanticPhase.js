"use strict";

// A try push's import in two phases (see tools/src/try_import.rs's `Phase`):
// once its history is imported, its review is there, saying that its
// semantic analysis is waiting or running (and how many translation units it
// compiles), and nothing caches it, until that's done; then it has its
// semantic analysis, and it's cached as pages are.  trydelta's 5ea5ed5ea5ed
// adds semantic_phase.cpp, and its analysis waits 15 s first (its
// slow-semantic; see tests/trydelta/import-try).

const PUSH = "5ea5ed5ea5ed";

function doc() {
  return frame.contentDocument;
}

async function statusOf(push) {
  return (await fetch(`/trydelta/try-status/hg/${push}`, { cache: "no-store" })).json();
}

async function cacheControlOf(push) {
  const response = await fetch(`/trydelta/try/hg/${push}`, { cache: "no-store" });
  return response.headers.get("Cache-Control");
}

add_task(async function test_TrySemanticPhase() {
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  const button = doc().querySelector(".try-import-button");
  ok(button, "The push's page has a button to import it");
  button.click();
  await waitForCondition(() => doc()?.querySelector(".try-import-status a"),
                         "which imports its history, and then the page links to its review",
                         500, 240);
  const status = await statusOf(PUSH);
  is(status.state, "analyzing", "while its semantic analysis is waiting or running");
  is(status.tus, 1, "of the one C++ file it changes");
  // (Once it's running, its status says its step and its progress, rather
  // than its log's last line; see the tools' `StepFollower`.)
  let running = status;
  for (let i = 0; i < 120 && running.state == "analyzing" && !running.step?.startsWith("try-import::semantic"); i++) {
    await TestUtils.sleep(250);
    running = await statusOf(PUSH);
  }
  is(running.step, "try-import::semantic: waiting 15 s, as its slow-semantic says",
     "Running, its status says its step, and its progress");
  ok(doc().querySelector(".try-import-status").textContent.includes("semantic analysis of 1 translation unit"),
     "which the page says");

  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  ok(doc().querySelector("#try-review"), "Its page is its review");
  ok(doc().querySelector('.tr-file[data-path="semantic_phase.cpp"]'), "with the file it adds");
  const pending = doc().querySelector(".tr-semantic-pending")?.textContent ?? "";
  ok(pending.includes("Its semantic analysis of 1 translation unit") &&
     /is (running|next|waiting)/.test(pending),
     "saying that its semantic analysis is still to come");
  // (Which the page keeps up to date, by the web server's import status; see
  // try-review.js's `watchSemanticPhase`.)
  await waitForCondition(
    () => doc().querySelector(".tr-semantic-pending")?.textContent.includes(", at try-import::semantic: waiting 15 s"),
    "and what it's doing, as the page keeps it up to date", 250, 60);
  // (Its semantic analysis's delta may be there already: the container's
  // index keeps the deltas of earlier runs, whose rebased heads are this
  // one's, which the fixtures' forged dates make the same.)
  is(await cacheControlOf(PUSH), "no-store", "and nothing caches it meanwhile");

  let state;
  for (let i = 0; i < 240; i++) {
    state = (await statusOf(PUSH)).state;
    if (state != "analyzing") {
      break;
    }
    await TestUtils.sleep(500);
  }
  is(state, "imported", "Its semantic analysis finishes");
  await waitForCondition(() => doc().querySelector(".tr-semantic-pending[data-done]"),
                         "and the page says a reload has it", 250, 40);
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  ok(!doc().querySelector(".tr-semantic-pending"), "Its review no longer says it's to come");
  ok(doc().querySelector(".tr-semantic-head"), "and has its semantic analysis");
  ok(doc().querySelector('.tr-file[data-path="semantic_phase.cpp"] [data-symbols*="SemanticPhase"]'),
     "its function's symbol among them");
  is(await cacheControlOf(PUSH), "max-age=120", "and it's cached as pages are");
});
