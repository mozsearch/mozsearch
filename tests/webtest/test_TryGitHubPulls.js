"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 240 (it imports a pull request)

// A pull request of a tree's own GitHub repository, as the trydelta tree's
// config says it imports them (`github_repo` and `github_pull_requests`; its
// API's answers are tests/github-api's fixtures): pull request #7, imported
// as the try push `github/7` (its commits are tests/trydelta/pulls/7's; see
// its import-try), whose review's URL, /trydelta/review/github/7, imports
// it, and is then its review, as its try page is; and DWIM of its GitHub URL.
// (See the tools' try_import.rs's `github_pull_requests` and the
// web-server's `github_review_page`.)

const HEAD = "7889b1d99165d769994cf9f385ce0c5e8787eeeb";

function doc() {
  return frame.contentDocument;
}

add_task(async function test_GitHubPullRequestReview() {
  await TestUtils.loadPath("/trydelta/review/github/7");
  if (!doc().querySelector("#try-review")) {
    ok(doc().querySelector(".try-import h2")?.textContent.includes("Pull request #7 of searchfox-tests/trydelta"),
       "A pull request's review URL, before it's imported, says which it is");
    ok(!doc().querySelector(".try-import-button") && doc().querySelector(".try-import-status"),
       "and imports it, saying how far that is");
  }
  await waitForCondition(() => doc()?.querySelector("#try-review[data-review-ready]"),
                         "until it's the review, at the same URL", 500, 240);
  is(doc().location.pathname, "/trydelta/review/github/7", "(still)");
  is(doc().querySelector("#query")?.value, "try-review:github/7", "the review of the pull request's import");
  ok(doc().querySelector('.tr-file[data-path="wiggle-notes.txt"]'), "with its changes");

  const status = await (await fetch("/trydelta/api/v1/try-status?push=github/7", { cache: "no-store" })).json();
  is(status.head, HEAD, "which is of its head, as GitHub's API has it");

  const heading = doc().querySelector(".tr-head h2");
  is(heading?.textContent, "Review of the pull request #7", "which says what it's of");
  is(heading?.querySelector("a")?.getAttribute("href"), "/trydelta/try/github/7", "and links to its try page");

  await TestUtils.loadPath("/trydelta/try/github/7");
  ok(doc().querySelector("#try-review"), "Its try page is its review too");

  is((await fetch("/trydelta/review/github/8", { cache: "no-store" })).status, 404,
     "A pull request GitHub doesn't know isn't found");
  is((await fetch("/tests/review/github/7", { cache: "no-store" })).status, 404,
     "nor is one in a tree which doesn't import them");

  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent("https://github.com/searchfox-tests/trydelta/pull/7")}`);
  is(doc().location.pathname, "/trydelta/review/github/7", "The pull request's GitHub URL goes to its review");

  const inventory = await (await fetch("/trydelta/try-imports/inventory", { cache: "no-store" })).json();
  ok(inventory.pushes.some(push => push.push == "github/7" && push.head == HEAD),
     "It's carried over to the next index's web server, as try pushes are");
});
