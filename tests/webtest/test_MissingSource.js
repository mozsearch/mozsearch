"use strict";

// The paths under `/{tree}/source/` which the tree doesn't have (anymore)
// say what became of them (see tools/src/format.rs's `format_missing_source`
// and tools/src/hyperblame/file_fate.rs), from the searchfox tree's history,
// mozsearch's own: in b548dba5 (2016), aws/ moved to infrastructure/aws/,
// with aws/attach-index-volume.py, and aws/indexer-startup.sh was deleted,
// its content moving to infrastructure/aws/index.sh and indexer-provision.sh.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_MissingSourceMoved() {
  await TestUtils.loadPath("/searchfox/source/aws/attach-index-volume.py#5");
  is(doc().location.pathname, "/searchfox/source/infrastructure/aws/attach-index-volume.py",
     "A moved file's old path goes to where it is");
  ok(doc().querySelector(".redirect-note")?.textContent.includes("aws/attach-index-volume.py"),
     "saying where it was");
  ok(doc().querySelector('.redirect-note a[href^="/searchfox/commit/b548dba5"]'),
     "and which commit moved it");
});

add_task(async function test_MissingSourceDeleted() {
  const response = await fetch("/searchfox/source/aws/indexer-startup.sh", { cache: "no-store" });
  is(response.status, 404, "A deleted file's path is a 404");
  await TestUtils.loadPath("/searchfox/source/aws/indexer-startup.sh");
  const page = doc().querySelector(".missing-source");
  ok(page?.textContent.includes("deleted in b548dba57185"), "saying when it was deleted");
  ok(page.querySelector('a.missing-source-last[href^="/searchfox/rev/"][href$="/aws/indexer-startup.sh"]'),
     "linking its last version");
  ok(page.querySelector('.missing-source-content a[href="/searchfox/source/infrastructure/aws/index.sh"]'),
     "and where its content went");
});

add_task(async function test_MissingSourceDirectory() {
  await TestUtils.loadPath("/searchfox/source/aws");
  is(doc().location.pathname, "/searchfox/source/infrastructure/aws",
     "A directory most of whose files moved goes there");
  ok(doc().querySelector(".redirect-note"), "saying it was redirected");
});

add_task(async function test_MissingSourceUnknown() {
  await TestUtils.loadPath("/searchfox/source/infrastructure/aws/indexr.sh");
  const page = doc().querySelector(".missing-source");
  ok(page?.querySelector('a[href="/searchfox/search?q=index.sh"]'),
     "A path its history never had suggests corrections of its name");
  ok(page.querySelector('a[href="/searchfox/source/infrastructure/aws"]'),
     "and links the nearest directory");
});
