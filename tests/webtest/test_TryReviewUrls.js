"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 240 (it imports pushes)

// A Phabricator revision's review URL (`/{tree}/review/phab/D123`; see the
// web-server's `review_page`), which stays the revision's whichever try push
// its latest diff is: the review of the code review bot's try push of its
// latest diff (by tests/code-review's fixtures: D8901's is abcdef012345,
// D9010's bead0bead0be, and D9012's the git push GIT_PUSH, as the bot's
// pushes to firefox-crb are), starting at the revision's patch; imported if
// it isn't, by the URL itself; or one push of it, which the URL pins.

const GIT_PUSH = "git/9012deadbeef9012deadbeef9012deadbeef9012";

function doc() {
  return frame.contentDocument;
}

async function review(path) {
  await TestUtils.loadPath(path);
  await waitForCondition(() => doc()?.querySelector("#try-review[data-review-ready]"),
                         `${path} is a review`, 500, 240);
}

add_task(async function test_TryReviewUrl() {
  await review("/trydelta/review/phab/D8901");
  is(doc().location.pathname, "/trydelta/review/phab/D8901", "A revision's review URL stays its URL");
  ok(doc().querySelector("#query")?.value.includes("abcdef012345"),
     "and is the review of its latest diff's try push");
  const patch = doc().querySelector('.tr-patch[data-phab="8901"]');
  ok(patch, "whose patches say which revisions they are");
  is(doc().querySelector("#try-review").dataset.focusedPatch, patch?.id,
     "and which starts at the revision's");
  is(patch?.querySelector(".tr-review-link")?.getAttribute("href"), "/trydelta/review/phab/D8901",
     "whose heading links to the revision's review URL");

  await review("/trydelta/review/phab/D8901/abcdef012345");
  ok(doc().querySelector("#query")?.value.includes("abcdef012345"),
     "A review URL can pin one of the revision's pushes");
});

add_task(async function test_TryReviewUrlImports() {
  await TestUtils.loadPath("/trydelta/review/phab/D9010");
  if (!doc().querySelector("#try-review")) {
    ok(doc().querySelector(".try-import-revision")?.textContent.includes("D9010's latest diff"),
       "A review URL whose push isn't imported says what it's of");
    ok(!doc().querySelector(".try-import-button") && doc().querySelector(".try-import-status"),
       "and imports it, as a person's request, saying how far that is");
  }
  await waitForCondition(() => doc()?.querySelector("#try-review[data-review-ready]"),
                         "until it's the review, at the same URL", 500, 240);
  is(doc().location.pathname, "/trydelta/review/phab/D9010", "(still)");
  ok(doc().querySelector("#query")?.value.includes("bead0bead0be"), "of the push");
});

// (A git push, `git/REV`, at `/{tree}/try/git/REV`, whose commits are
// tests/trydelta's try/git/REV's; see its import-try.)
add_task(async function test_TryReviewUrlGit() {
  await review("/trydelta/review/phab/D9012");
  is(doc().location.pathname, "/trydelta/review/phab/D9012", "A revision whose latest push is a git push");
  is(doc().querySelector("#query")?.value, `try-review:${GIT_PUSH}`, "has its review");
  ok(doc().querySelector('.tr-file[data-path="wobble-directions.txt"]'), "with its changes");
  const heading = doc().querySelector(".tr-head h2");
  is(heading?.textContent, "Review of the try push 9012deadbeef", "named by its revision");
  is(heading?.querySelector("a")?.getAttribute("href"), `/trydelta/try/${GIT_PUSH}`,
     "and linking to its try page, a git push's");

  await review(`/trydelta/try/${GIT_PUSH}`);
  is(doc().querySelector("#query")?.value, `try-review:${GIT_PUSH}`, "Its try page is its review too");
  await review(`/trydelta/review/phab/D9012/${GIT_PUSH}`);
  is(doc().location.pathname, `/trydelta/review/phab/D9012/${GIT_PUSH}`, "as is the review URL pinning it");

  for (const path of ["/trydelta/try/git/9012deadbeef", "/trydelta/review/phab/D9012/git/9012deadbeef",
                      `/trydelta/review/phab/D9012/hg/${GIT_PUSH.slice(4)}`]) {
    is((await fetch(path, { cache: "no-store" })).status, 404, `${path} isn't a push's (by its whole revision)`);
  }

  const inventory = await (await fetch("/trydelta/try-imports/inventory", { cache: "no-store" })).json();
  ok(inventory.pushes.some(push => push.push == GIT_PUSH), "It's carried over to the next index's web server");
});

add_task(async function test_TryReviewUrlMissing() {
  await TestUtils.loadPath("/trydelta/review/phab/D4242");
  ok(doc().querySelector(".review-missing")?.textContent.includes("no try push of D4242"),
     "A revision without a try push's review URL says so");
  await TestUtils.loadPath("/trydelta/review/phab/D12x");
  ok(!doc().querySelector(".review-missing"), "A URL without a revision's number isn't one");
});

// (DWIM: a Phabricator revision's URL and its number are its review, at once;
// see test_DwimLinks.js.)
add_task(async function test_TryReviewUrlDwim() {
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent("https://phabricator.services.mozilla.com/D8901")}`);
  is(doc().location.pathname, "/trydelta/review/phab/D8901", "A revision's URL goes to its review");
  await TestUtils.loadPath(`/trydelta/search?q=D8901`);
  is(doc().location.pathname, "/trydelta/review/phab/D8901", "as does its number");
  await waitForCondition(() => doc().querySelector(".dwim-searched"), "which offers the search instead");
});
