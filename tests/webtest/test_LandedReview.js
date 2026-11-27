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

// A heading's `#` (see the tools' try_review.rs's `anchor_html`), shown on
// hovering over the heading: a link to its part (its hunk's, definition's or
// file's), to copy, or to go to, a history entry, after which the page's URL
// is the link's, and Back returns to where the page was.
add_task(async function test_ReviewAnchors() {
  await TestUtils.loadPath("/searchfox/explore/bug/1975601");
  await waitForCondition(() => doc()?.querySelector("#try-review[data-review-ready]"),
                         "The bug's review is there", 500, 120);
  const hunk = doc().querySelector(".tr-hunk:not(.tr-hunk-folded)");
  const anchor = hunk?.querySelector(":scope > .tr-hunk-head > a.tr-hash");
  is(anchor?.getAttribute("href"), `#${hunk?.id}`, "A hunk's heading has a link to the hunk");
  is(anchor?.textContent, "#", "a #");
  ok(parseFloat(frame.contentWindow.getComputedStyle(anchor).opacity) < 1,
     "which isn't shown (or, on a touch screen, faintly) until its heading is hovered over");
  // (To the left of the heading's text, on its line, out of its flow.)
  const lines = hunk.querySelector(":scope > .tr-hunk-head > .tr-hunk-lines");
  const at = anchor.getBoundingClientRect();
  const text = lines.getBoundingClientRect();
  ok(at.right <= text.left + 1 && at.width > 0, `left of the heading's text (${at.right} <= ${text.left})`);
  ok(Math.abs(at.top - text.top) < text.height, `on its line (${at.top} by ${text.top})`);
  is(frame.contentWindow.getComputedStyle(anchor).position, "absolute", "out of its flow");
  for (const [part, head] of [["tr-def", "tr-def-head"], ["tr-file", "tr-file-head"]]) {
    const element = doc().querySelector(`.${part}[id]`);
    is(element?.querySelector(`:scope > .${head} > a.tr-hash`)?.getAttribute("href"), `#${element?.id}`,
       `and so does a .${part}'s heading`);
  }
  // (A folded hunk's heading is its definition's.)
  const folded = doc().querySelector(".tr-hunk-folded");
  const def = folded?.closest(".tr-def");
  ok(def?.querySelector(":scope > .tr-def-head > a.tr-hash")?.getAttribute("href") == `#${def?.id}`,
     "A folded hunk's link is its definition's");

  const before = doc().location.href;
  anchor.click();
  is(doc().location.hash, `#${hunk.id}`, "Clicking it makes the page's URL the hunk's");
  ok(hunk.matches(":target"), "and goes to it");
  frame.contentWindow.history.back();
  await waitForCondition(() => doc().location.href == before, "Back returns to where the page was", 100, 50);
});
