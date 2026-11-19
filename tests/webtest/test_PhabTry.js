"use strict";

// The data API's `phab-try` (see the tools' code_review.rs): the latest try
// push the code review bot made of a Phabricator revision's diffs, for review
// dashboards on other origins, which can't ask the bot's API themselves (by
// tests/code-review's fixture of the bot's API, which the webtest config's
// `code_review_url` is).

add_task(async function test_PhabTry() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const win = frame.contentWindow;
  const get = query => win.fetch(`/trydelta/api/v1/phab-try?${query}`, { cache: "no-store" });

  const response = await get("revision=D333833");
  is(response.status, 200, "A revision's latest try push is found");
  is(response.headers.get("Access-Control-Allow-Origin"), "*", "for other origins' pages too");
  const push = await response.json();
  is(push.push, "eb82baaeebe8569e0a6c37fa9f12289a9fe7e850", "the latest of its diffs' pushes");
  is(push.diff_phid, "PHID-DIFF-pwvkmz6doimuhqweswq4", "with its diff (Phabricator's PHID)");
  is(push.created, "2026-11-03T21:55:52.781404Z", "and when the bot made it");
  is(JSON.stringify(await (await get("revision=333833")).json()), JSON.stringify(push),
     "(by its number alone too)");

  is(await (await get("revision=1")).json(), null, "A revision the bot didn't push has none");
  is((await get("revision=D")).status, 422, "and a revision must be a number");
});
