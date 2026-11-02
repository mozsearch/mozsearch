"use strict";

// Crash sets' ingestion (see tools/src/crash_sets.rs's `request_ingestion`
// and web-server.rs's `crash-sets` routes): a crash signature's view links to
// the crash sets' form, its fields the signature's search, which POSTs (with
// an optional API token, in its body) asking for the crash reports' fetch,
// whose status page then goes to the set's view; a token which can see
// protected data is refused.  crash-stats is tests/crash-stats/'s stand-in
// (see webtest-config.json's `crash_stats_api`), whose search finds
// tests/crash-reports/'s 2 crashes.
const SIGNATURE = "tools::blame::find_phab_rev";

function doc() {
  return frame.contentDocument;
}

// Submits the form (as a person's click does, a navigation), with `token`:
// to its ingestion's status, or if it's done already, the set's view.
async function submit(token) {
  const form = doc().querySelector("form.crash-set-form");
  form.querySelector('input[name="token"]').value = token;
  form.querySelector(".crash-set-submit").click();
  await TestUtils.waitForCondition(() => doc().querySelector(".crash-set-status, .crash-set-view"),
                                   "The form goes to its ingestion's status");
}

add_task(async function test_CrashSetIngest() {
  await TestUtils.loadQuery("searchfox", `crash-signature:'${SIGNATURE}'`);
  const link = doc().querySelector("a.crash-set-new");
  ok(link, "A crash signature's view links to the crash sets' form");
  const url = new URL(link.href);
  await TestUtils.loadPath(url.pathname + url.search);

  const field = name => doc().querySelector(`form.crash-set-form [name="${name}"]`);
  is(field("signature")?.value, SIGNATURE, "The form's search is the signature's");
  ok(/^\d{4}-\d{2}-\d{2}$/.test(field("from").value) && /^\d{4}-\d{2}-\d{2}$/.test(field("through").value),
     "the last week's");
  is(field("limit").value, "100", "the latest 100");
  is(field("token").type, "password", "with a token's field, hidden");
  is(doc().querySelector("form.crash-set-form").method, "post", "which it POSTs, rather than putting in a URL");

  // A token which can see protected data is refused.
  await submit("privileged-test-token");
  await TestUtils.waitForCondition(() => doc().querySelector(".crash-set-status")?.dataset.state == "failed",
                                   "A token which can see protected data fails", 100, 100);
  ok(doc().querySelector(".crash-set-error")?.textContent.includes("can see protected crash data"),
     "saying why");
  ok(!doc().location.href.includes("privileged-test-token"), "(The token isn't in the URL)");

  // Without a token, the set's fetched, and its view shows.
  await TestUtils.loadPath(url.pathname + url.search);
  await submit("");
  await TestUtils.waitForCondition(() => doc().querySelector(".crash-set-view"),
                                   "Once fetched, the status page goes to the set's view", 100, 100);
  const notice = doc().querySelector(".crash-set-view .crash-notice").textContent;
  ok(notice.includes(`2 crash reports from crash-stats' search for the signature ${SIGNATURE} (Firefox,`),
     `The set is the search's crash reports: ${notice}`);
  is(doc().querySelectorAll(".crash-set-crash").length, 2, "both of them");
  ok(new URLSearchParams(doc().location.search).get("q").startsWith("crash-set:"), "at the set's query");
});

add_task(async function test_CrashSetFormProblems() {
  // (The form says what's wrong, with its fields as they were.)
  await TestUtils.loadPath(`/searchfox/crash-sets/new?signature=${encodeURIComponent(SIGNATURE)}`);
  doc().querySelector('form.crash-set-form [name="from"]').value = "2020-01-01";
  doc().querySelector('form.crash-set-form [name="through"]').value = "2020-01-07";
  doc().querySelector('form.crash-set-form .crash-set-submit').click();
  await TestUtils.waitForCondition(() => doc().querySelector(".crash-set-form-notice"),
                                   "A search with old dates is refused");
  ok(doc().querySelector(".crash-set-form-notice").textContent.includes("within the last 30 days"), "saying why");
  is(doc().querySelector('form.crash-set-form [name="signature"]').value, SIGNATURE, "with its fields");
  is(doc().querySelector('form.crash-set-form [name="token"]').value, "", "but not a token");
});
