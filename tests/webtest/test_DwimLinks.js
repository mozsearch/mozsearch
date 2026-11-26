"use strict";

// DWIM rules for other tools' pages (see tools/src/query/dwim.toml and
// `format::dwim_lookup`), which a search in a tree searching with queries
// (trydelta) goes to at once: a Bugzilla bug's commits' explore page, a
// Phabricator revision's review (in a tree importing try pushes, as it hasn't
// landed; see test_TryReviewUrls.js), a GitHub commit's page (in another tree, the
// searchfox tree, whose repository has it), and a try push's page from
// Treeherder's (but not another repository's push, which is searched for),
// or after "try" (which Firefox's address bar searches for, rather than going
// to a pasted URL), its Treeherder URL, its revision, its hg URL, or a
// Phabricator revision, whose try push is the code review bot's of its latest
// diff (by its API, tests/code-review's fixture); a git try push's page from
// its commit's URL in firefox-crb (or firefox-dev, unless a tree has it) or
// Treeherder's; and the route an hg commit goes to.

function doc() {
  return frame.contentDocument;
}

async function search(input) {
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(input)}`);
  return doc().location.pathname;
}

add_task(async function test_DwimLinks() {
  is(await search("https://bugzilla.mozilla.org/show_bug.cgi?id=1517978"), "/trydelta/explore/bug/1517978",
     "A bug goes to its commits' explore page");
  is(await search("https://bugzil.la/1517978"), "/trydelta/explore/bug/1517978", "as does its short URL");
  is(await search("https://phabricator.services.mozilla.com/D123456"), "/trydelta/review/phab/D123456",
     "A Phabricator revision goes to its review, as it hasn't landed");
  ok(doc().querySelector(".dwim-searched"), "which offers the search instead");
  ok(doc().querySelector(".review-missing"), "and says that the code review bot has no try push of it");

  const rev = "50df75fc5253890347bfac70037bc7ad42316862";
  is(await search(`https://github.com/mozsearch/mozsearch/commit/${rev}`), `/searchfox/commit/${rev}`,
     "A GitHub commit goes to its page, in the tree whose repository has it");

  is(await search("https://treeherder.mozilla.org/jobs?repo=try&revision=deadbeefdead"), "/trydelta/try/hg/deadbeefdead",
     "A try push on Treeherder goes to its page");
  is(await search("https://treeherder.mozilla.org/jobs?repo=autoland&revision=deadbeefdead"), "/trydelta/query/default",
     "but another repository's push is searched for");
  for (const input of ["try https://treeherder.mozilla.org/jobs?repo=try&revision=deadbeefdead",
                       "try deadbeefdead", "try https://hg.mozilla.org/try/rev/deadbeefdead"]) {
    is(await search(input), "/trydelta/try/hg/deadbeefdead", `"${input}" goes to the try push's page`);
  }
  const push = "eb82baaeebe8569e0a6c37fa9f12289a9fe7e850";
  for (const input of ["try https://phabricator.services.mozilla.com/D333833", "try D333833"]) {
    is(await search(input), `/trydelta/try/hg/${push}`,
       `"${input}" goes to the code review bot's try push of the revision's latest diff`);
  }
  is(await search("try D1"), "/trydelta/query/default", "but a revision it didn't push is searched for");

  // (Git try pushes, `git/REV`: the code review bot's pushes to firefox-crb,
  // and firefox-dev's branches'.)
  const git = "9012deadbeef9012deadbeef9012deadbeef9012";
  is(await search("try D9012"), `/trydelta/try/git/${git}`,
     "The code review bot's latest push of a revision can be a git push, to firefox-crb");
  for (const input of [`https://github.com/mozilla-firefox/firefox-crb/commit/${git}`,
                       `try https://github.com/mozilla-firefox/firefox-dev/tree/${git}`,
                       `https://treeherder.mozilla.org/jobs?repo=firefox-crb&revision=${git}`]) {
    is(await search(input), `/trydelta/try/git/${git}`, `"${input}" goes to the git push's page`);
  }
  is(await search(`https://github.com/mozilla-firefox/firefox-dev/commit/${rev}`), `/searchfox/commit/${rev}`,
     "but a commit a tree has goes to its page");
  is(await search("https://github.com/mozilla-firefox/firefox-crb/tree/development/code-review/D123456-654321"),
     "/trydelta/review/phab/D123456", "and a revision's branch in firefox-crb to its review");

  // (After ". ", which Firefox's address bar searches for too, every rule
  // applies, and the rest is what's searched for.)
  is(await search(". https://bugzil.la/1517978"), "/trydelta/explore/bug/1517978",
     'After ". ", a bug goes to its commits\' explore page too');
  is(await search(". try deadbeefdead"), "/trydelta/try/hg/deadbeefdead", 'as do "try "\'s rules');
  is(await search(". DwimNothingHere"), "/trydelta/query/default", "Without a rule, it's searched for");
  const params = new URLSearchParams(doc().location.search);
  is(params.get("q"), "DwimNothingHere", 'without the ". "');
  ok(!params.has("input"), "which is all of the input");

  // (The "hg-commit" rule's route, which nginx sends to the web server: the
  // searchfox tree's repository isn't hg's, so it doesn't know the revision.)
  const hg = await frame.contentWindow.fetch("/searchfox/hgcommit/deadbeefdead", { cache: "no-store" });
  is(hg.status, 404, "An hg commit the tree doesn't have isn't found");
  is(await hg.text(), "Not found", "by the web server, rather than nginx");
});
