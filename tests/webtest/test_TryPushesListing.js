"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 240 (it imports a push)

// A tree importing try pushes' /explore/ page: its imported pushes, each
// linked to its review (the reviews someone could do now), with facets, and
// weighed as landings are with `interesting=1`; and its branch's recent
// landings with `landed=1` (see the tools' cmd_landings.rs, and
// test_Landings.js).

const PUSH = "0123456789ab";

function doc() {
  return frame.contentDocument;
}

add_task(async function test_TryPushesListing() {
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"), "The push is imported", 500, 240);
  }

  await TestUtils.loadPath("/trydelta/explore/");
  await waitForCondition(() => doc()?.querySelector("#landings"), "The tree's explore page", 500, 120);
  is(doc().querySelector("#query")?.value, "try-pushes:7", "is its try pushes'");
  is(doc().querySelector("#landings h2")?.textContent, "Try pushes to review", "to review");
  const push = doc().querySelector(`.landing[data-path="push/${PUSH}"]`);
  ok(push, "An imported push is listed");
  is(push?.querySelector(".landing-link")?.getAttribute("href"), `/trydelta/try/hg/${PUSH}`,
     "linked to its review");
  is(push?.querySelector(".landing-link")?.textContent, `Try push ${PUSH}`, "by its name");
  ok(push?.querySelector(".landing-commits li"), "with its commits");
  ok(doc().querySelector('.facet[data-facet="type"]'), "Its facets are its pushes' files'");
  is(doc().querySelector(".landings-other")?.getAttribute("href"), "/trydelta/explore/?days=7&landed=1",
     "It links to the branch's landings");

  await TestUtils.loadPath("/trydelta/explore/?interesting=1");
  await waitForCondition(() => doc()?.querySelector("#landings"), "The interesting ones", 500, 120);
  is(doc().querySelector("#query")?.value, "interesting-try-pushes:7", "Its pushes can be weighed");
  ok(doc().querySelector(`.landing[data-path="push/${PUSH}"]`), "(the push among them)");

  await TestUtils.loadPath("/trydelta/explore/?days=7&landed=1");
  await waitForCondition(() => doc()?.querySelector("#landings"), "The branch's landings", 500, 120);
  is(doc().querySelector("#query")?.value, "landings:7", "are its landings");
  is(doc().querySelector(".landings-other")?.getAttribute("href"), "/trydelta/explore/?days=7",
     "which link back to its try pushes");
});
