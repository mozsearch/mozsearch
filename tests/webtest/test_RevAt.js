"use strict";

// The head a tree's branch had at a time, roughly (the `rev-at:` query
// term's; see tools/src/cmd_pipeline/cmd_rev_at.rs and the commit index's
// `by-landed`), which a search for it goes through to: the tree's files at
// that revision.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_RevAt() {
  await TestUtils.loadQuery("trydelta", "rev-at:2100-01-01");
  const rev = doc().querySelector(".rev-at-rev");
  ok(rev?.getAttribute("href").startsWith("/trydelta/rev/"), "A time's head is the tree's revision then");
  ok(doc().querySelector(".rev-at").textContent.includes("the last commit of its main line which had landed by then"),
     "roughly, which it says");

  await TestUtils.loadQuery("trydelta", "rev-at:1970-01-02");
  ok(doc().querySelector(".rev-at")?.textContent.includes("has no commits which had landed by 1970-01-02 23:59:59 UTC"),
     "A day before the tree's commits has none (its end)");

  await TestUtils.loadQuery("trydelta", "rev-at:yesterday");
  ok(doc().querySelector(".query-problem")?.textContent.includes("isn't a time"), "Something else isn't a time");

  // (A search for it goes there at once.)
  const href = rev.getAttribute("href");
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent("rev-at:2100-01-01")}`);
  is(doc().location.pathname, href, "A search for a time goes to the tree's files then");
});
