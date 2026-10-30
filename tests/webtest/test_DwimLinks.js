"use strict";

// DWIM rules for other tools' pages (see tools/src/query/dwim.toml and
// `format::dwim_lookup`), which a search in a tree searching with queries
// (trydelta) goes to at once: a Bugzilla bug's and a Phabricator revision's
// commits' explore pages, a GitHub commit's page (in another tree, the
// searchfox tree, whose repository has it), and a try push's page from
// Treeherder's (but not another repository's push, which is searched for).

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
  is(await search("https://phabricator.services.mozilla.com/D123456"), "/trydelta/explore/phab/D123456",
     "A Phabricator revision goes to its commits' explore page");
  ok(doc().querySelector(".dwim-searched"), "which offers the search instead");

  const rev = "50df75fc5253890347bfac70037bc7ad42316862";
  is(await search(`https://github.com/mozsearch/mozsearch/commit/${rev}`), `/searchfox/commit/${rev}`,
     "A GitHub commit goes to its page, in the tree whose repository has it");

  is(await search("https://treeherder.mozilla.org/jobs?repo=try&revision=deadbeefdead"), "/trydelta/try/hg/deadbeefdead",
     "A try push on Treeherder goes to its page");
  is(await search("https://treeherder.mozilla.org/jobs?repo=autoland&revision=deadbeefdead"), "/trydelta/query/default",
     "but another repository's push is searched for");
});
