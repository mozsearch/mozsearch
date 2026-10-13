"use strict";

// The spechist tree imports try pushes (see tools/src/try_import.rs), whose
// hook makes them from tests/spechist/try/ (see tests/spechist/import-try):
// abcdef012345 is a push of two commits on top of the tree's head, the first
// changing cite.cpp, for D5678.

const PUSH = "abcdef012345";

function doc() {
  return frame.contentDocument;
}

async function statusOf(push) {
  return (await fetch(`/spechist/try-status/${push}`)).json();
}

add_task(async function test_TryImport() {
  await TestUtils.loadPath(`/spechist/try/${PUSH}`);
  ok(doc().querySelector(".try-import-status")?.textContent.includes("hasn't imported"),
     "A try push's page says the tree hasn't imported it");
  is((await statusOf(PUSH)).state, "notImported", "and so does its status");

  const button = doc().querySelector(".try-import-button");
  ok(button, "The page has a button to import it");
  button.click();
  await waitForCondition(() => doc()?.querySelector(".explore-page"),
                         "which imports it, and then the page is its explore page",
                         500, 240);
  is(doc().querySelector(".explore-page h2")?.textContent, `Try push ${PUSH}`,
     "titled by the push");
  const commits = [...doc().querySelectorAll(".explore-commit-list .explore-commit")];
  is(commits.length, 2, "with the push's commits");
  ok(commits[0].textContent.includes("Make Frob frob twice"), "oldest first");
  ok(doc().querySelector('.explore-file a[href*="cite.cpp"]'),
     "and the files they changed");
  is((await statusOf(PUSH)).state, "imported", "Its status says it's imported");

  // The tree's views have the push's revisions' history, ex: the blame of the
  // file its first commit changed, at that commit.
  const rev = commits[0].querySelector('a[href*="/commit/"]').getAttribute("href").split("/").pop();
  await TestUtils.loadPath(`/spechist/rev/${rev}/cite.cpp`);
  const info = frame.contentWindow.BLAME_INFO;
  ok(info?.dataUrl?.includes(`/rev-hyperblame/${rev}/`),
     "The push's revisions' pages have their token-centric blame");
  const blameCommits = await (await fetch(`${info.dataUrl}/commits.json`)).json();
  ok(blameCommits.some(commit => commit.header.includes("Make Frob frob twice")),
     "including the lines the push changed");

  // Its Phabricator revision's explore page has its commit, from the try
  // pushes' commit index.
  await TestUtils.loadPath("/spechist/explore/phab/D5678");
  ok(doc().querySelector(`.explore-other-branches a[href="/spechist/commit/${rev}"]`),
     "The push's Phabricator revision's explore page has its commit");
});

add_task(async function test_TryImportFailure() {
  // A push the hook can't fetch fails, with the hook's output's last lines.
  const missing = "0123456789ab";
  const response = await fetch(`/spechist/try/${missing}`, { method: "POST" });
  ok(response.ok, "Asking for a push's import");
  let status;
  for (let i = 0; i < 240; i++) {
    status = await statusOf(missing);
    if (status.state == "failed") {
      break;
    }
    await TestUtils.sleep(500);
  }
  is(status.state, "failed", "which fails if the push can't be fetched");
  ok(status.message.includes("there's no try push"), "with what the hook said");
  ok(!status.retry, "and can't be asked for again right away");

  await TestUtils.loadPath(`/spechist/try/${missing}`);
  ok(doc().querySelector(".try-import-error")?.textContent.includes("there's no try push"),
     "Its page says why it failed");
  ok(!doc().querySelector(".try-import-button"), "without a button to try again yet");
});
