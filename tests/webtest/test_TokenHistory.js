"use strict";

// What the history says about the removals of a token searched for (see
// tools/src/hyperblame/token_history.rs), from the "searchfox" tree's history
// of mozsearch itself: its 2022 upgrade to clap v3 removed `structopt`, whose
// search notes that, and what replaced it (clap), and the `token-history:`
// view shows its journal.

function doc() {
  return frame.contentDocument;
}

const UPGRADE = "0f2a4938884c2ce7e2637363c5577f095ef0d074";

add_task(async function test_TokenHistoryNote() {
  await TestUtils.loadQuery("searchfox", "structopt");

  const note = doc().querySelector(".token-history-note");
  ok(note, "The search notes the token's removals, as the results are few");
  ok(note.querySelector(".token-history-summary").textContent.includes("instances of structopt were removed"),
     "how many of its instances were removed");
  const commit = note.querySelector(`.token-history-commits a[href="/searchfox/commit/${UPGRADE}"]`);
  ok(commit, "by which commits");
  ok(commit.closest("li").textContent.includes("Upgrade to clap v3"), "with their messages");

  const replacement = note.querySelector(".token-history-replacement a");
  is(replacement?.textContent, "clap", "It was replaced by clap");
  is(replacement?.getAttribute("href"), "/searchfox/query/default?q=clap", "which it offers to search for");

  const more = note.querySelector(".token-history-more a");
  ok(more.getAttribute("href").includes("token-history:structopt"), "It links to the token's history");
  ok([...note.querySelectorAll(".token-history-more a")].some(a => a.textContent == "StructOpt"),
     "and to its case variants'");
});

add_task(async function test_TokenHistoryNoNote() {
  // (A common token has many results, more than its removals removed.)
  await TestUtils.loadQuery("searchfox", "crossref");
  ok(!doc().querySelector(".token-history-note"), "A living token's search has no note");
});

add_task(async function test_TokenHistoryView() {
  await TestUtils.loadQuery("searchfox", "token-history:structopt case:sensitive");

  const view = doc().querySelector(".token-history-view");
  is(view.querySelector("h2")?.textContent, "The history of structopt", "The view is the token's");
  ok(view.querySelector(".token-history-summary"), "with its removals");
  const row = view.querySelector(`.token-history-records a[href="/searchfox/commit/${UPGRADE}"]`)?.closest("tr");
  ok(row, "and its journal's records");
  ok(row.querySelector(".token-history-removed").textContent.includes("−"), "with what they removed");
  ok(row.textContent.includes("Upgrade to clap v3"), "and their messages");
  ok([...view.querySelectorAll(".token-history-years td:first-child")].some(td => td.textContent == "2022"),
     "and by year");
});

add_task(async function test_TokenHistoryWithoutHistory() {
  // (The "tests" tree has no history.)
  await TestUtils.loadQuery("tests", "token-history:Foo");
  ok(doc().querySelector(".token-history-view").textContent.includes("no token-centric history"),
     "A tree without a history says so");
});

add_task(async function test_TokenHistoryApi() {
  // (The data API's token-history route, for remote servers.)
  const response = await fetch("/searchfox/api/v1/token-history?token=structopt", { cache: "no-store" });
  is(response.status, 200, "The data API has token histories");
  const history = await response.json();
  is(history.token, "structopt", "of the token");
  ok(history.removed > 50, "with how many of its instances were removed");
  ok(history.commits.some(commit => commit.rev == UPGRADE), "and by which commits");
  const none = await fetch("/searchfox/api/v1/token-history?token=Thingamajigger", { cache: "no-store" });
  is(await none.text(), "null", "A token without a journal has none");
});
