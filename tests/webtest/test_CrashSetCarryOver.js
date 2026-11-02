"use strict";

// Crash sets' lifetimes and carry-over (see tools/src/crash_sets.rs's
// `carry_over` and infrastructure/aws/carry-over-crash-sets.py): a set's view
// notes its use (its beacon, `crash-sets/touch/ID`), and the next index's
// carry-over asks the web server which sets to carry over within a budget
// (`crash-sets/inventory?bytes=`) and for their files (`crash-sets/files`),
// which only the indexer may, not requests through the load balancer (with
// forwarding headers).  The set is one this ingests, as test_CrashSetIngest
// does (tests/crash-stats/'s stand-in's 2 crashes).
const SIGNATURE = "tools::blame::find_phab_rev";

function doc() {
  return frame.contentDocument;
}

add_task(async function test_CrashSetCarryOver() {
  await TestUtils.loadPath(`/searchfox/crash-sets/new?signature=${encodeURIComponent(SIGNATURE)}`);
  doc().querySelector(".crash-set-form .crash-set-submit").click();
  await TestUtils.waitForCondition(() => doc().querySelector(".crash-set-view"),
                                   "The set's fetched, and its view shows", 100, 100);
  const set = new URLSearchParams(doc().location.search).get("q").replace(/^crash-set:/, "");
  ok(/^[0-9a-f]{16}$/.test(set), `at its query: ${set}`);
  ok([...doc().querySelectorAll("script")].some(script => script.textContent.includes(`/searchfox/crash-sets/touch/${set}`)),
     "whose view notes its use");

  const touch = id => fetch(`/searchfox/crash-sets/touch/${id}`, { method: "POST", cache: "no-store" });
  is((await touch(set)).status, 204, "A set's use is noted");
  is((await touch("0000000000000000")).status, 404, "but not one the store doesn't have");

  const inventory = async (bytes, headers = {}) =>
    fetch(`/searchfox/crash-sets/inventory?bytes=${bytes}`, { cache: "no-store", headers });
  const all = await (await inventory(1e12)).json();
  const carried = all.sets.find(entry => entry.id == set);
  ok(carried?.bytes > 0, "The carry-over has the set, with its files' size");
  ok(Math.abs(carried.touched - Date.now() / 1000) < 600, "and when it was last used");
  ok(all.files.includes(`sets/${set}.json`), "and its manifest");
  is(all.bytes, all.sets.reduce((bytes, entry) => bytes + entry.bytes, 0), "and how many bytes they take");
  is((await (await inventory(0)).json()).sets.length, 0, "Within no budget, nothing");

  const files = await fetch("/searchfox/crash-sets/files", {
    method: "POST",
    cache: "no-store",
    body: [`sets/${set}.json`, "../../etc/passwd", "cap"].join("\n"),
  });
  const lines = (await files.text()).trim().split("\n").map(line => JSON.parse(line));
  is(lines.length, 1, "Of the files asked for, only the store's sets' and crashes'");
  is(lines[0].path, `sets/${set}.json`, "by their paths");
  const manifest = JSON.parse(atob(lines[0].data));
  is(manifest.search.signature, SIGNATURE, "their contents in base64");
  is(manifest.crashes.length, 2, "(the set's manifest)");

  // (Only for the indexer.)
  is((await inventory(1e12, { "X-Forwarded-For": "203.0.113.7" })).status, 403,
     "Not for requests through the load balancer (nginx)");
  is((await inventory(1e12, { "Fastly-Client-IP": "203.0.113.7" })).status, 404,
     "nor those from Fastly (the web-server)");
});
