"use strict";

// The data API's `phab-try` (see the tools' code_review.rs): the latest try
// push the code review bot made of a Phabricator revision's diffs (to hg's try,
// or to firefox-crb, its GitHub repository for them), for review
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

  is(push.repository, "try", "and where: hg's try repository");

  // (The bot also pushes diffs to firefox's GitHub repository for them,
  // firefox-crb, whose pushes are git commits, git try pushes: D9012's latest
  // diff's is there, after one on try; and a GitHub pull request of the same
  // number, which isn't the revision's, is newer.)
  const crb = await (await get("revision=D9012")).json();
  is(`${crb.push} ${crb.repository}`, "git/9012deadbeef9012deadbeef9012deadbeef9012 mozilla-firefox_firefox-crb",
     "A revision's latest push can be the bot's on firefox-crb, a git push");

  is(await (await get("revision=1")).json(), null, "A revision the bot didn't push has none");
  is((await get("revision=D")).status, 422, "and a revision must be a number");
});
