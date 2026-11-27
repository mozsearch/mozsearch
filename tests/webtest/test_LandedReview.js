"use strict";
// webtest-timeout: 120 (it renders reviews)

// A landed stack's review: a bug's (or Phabricator revision's) explore page
// is the review view of the commits which mention it, as they landed (the
// `review:` term's; see the tools' try_review.rs's `landed_review_name`),
// with their blame, and a link to the explore page's outline view
// (`?view=outline`; see test_Explore.js).  Bug 1975601's 4 parts landed
// together.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_LandedReview() {
  await TestUtils.loadPath("/searchfox/explore/bug/1975601");
  await waitForCondition(() => doc()?.querySelector("#try-review[data-review-ready]"),
                         "A bug's explore page is the review of its commits", 500, 120);
  is(doc().location.pathname, "/searchfox/explore/bug/1975601", "at its URL");
  is(doc().querySelector("#query")?.value, "review:bug/1975601", "which is its query's");
  is(doc().querySelector(".tr-head h2")?.textContent, "Review of bug 1975601", "of the bug");

  const summaries = [...doc().querySelectorAll(".tr-patch .tr-patch-summary")].map(s => s.textContent);
  is(summaries.length, 4, "with its 4 commits");
  for (let i = 0; i < 4; i++) {
    ok(summaries[i]?.includes(`Part ${i + 1}:`), `Part ${i + 1} is patch ${i + 1}`);
  }
  const modes = [...doc().querySelectorAll(".tr-head .then-now-modes button")].map(b => b.textContent);
  is(modes.join(", "), "Before, After, Before & After", "Its modes are before and after each commit");
  ok(doc().querySelector(".now-code .tr-line .blame-strip[data-hb-ctx]") &&
     doc().querySelector(".then-code .tr-line .blame-strip[data-hb-ctx]"),
     "Its lines have their blame, then and now");

  const note = doc().querySelector(".tr-landed-of");
  ok(note?.querySelector('a.tr-landed-key[href="https://bugzilla.mozilla.org/show_bug.cgi?id=1975601"]'),
     "It links to the bug");
  is(note?.querySelector("a.tr-outline-view")?.getAttribute("href"), "/searchfox/explore/bug/1975601?view=outline",
     "and to its outline view");
  is(doc().querySelector(".breadcrumbs .tr-crumbs a")?.textContent, "bug 1975601", "The breadcrumbs name it");

  // (A hunk's heading links to its lines in its commit, in the stack's
  // context.)
  const link = doc().querySelector(".tr-hunk-head a[href*='/source/'], .tr-def-head a[href*='/source/']");
  const href = link?.getAttribute("href") ?? "";
  ok(/^\/searchfox\/explore\/bug\/1975601\/source\/[0-9a-f]{40}\//.test(href), `A hunk links to its lines (${href})`);
  const response = await fetch(href, { cache: "no-store" });
  ok(response.ok && /\/searchfox\/rev\/[0-9a-f]{40}\/.*\?stack=explore\/bug\/1975601/.test(response.url),
     `in its commit's file, in the stack's context (${response.url})`);

  // (The outline view is still there.)
  await TestUtils.loadPath("/searchfox/explore/bug/1975601?view=outline");
  is(doc().querySelectorAll(".explore-commit-summary").length, 4, "The outline view has the bug's commits");

  // (A bug no commit mentions has no review.)
  await TestUtils.loadPath("/searchfox/explore/bug/9999999");
  ok(doc().body.textContent.includes("No commit of this tree's mentions 9999999"),
     "A bug no commit mentions says so");
});
