"use strict";
// webtest-timeout: 120 (it weighs commits)

// A tree's /explore/ page: its recent landings (the last week's up to its
// last landing), by stack (each bug's commits), newest first, each linked to
// its review (its bug's explore page; see test_LandedReview.js), with facets;
// and, with `interesting=1`, weighed by how much more their reviews show than
// a line diff does (see the tools' cmd_landings.rs).  The searchfox tree's
// history is mozsearch's own, whose recent commits are hyperblame's parts.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_Landings() {
  await TestUtils.loadPath("/searchfox/explore/");
  await waitForCondition(() => doc()?.querySelector("#landings"), "The tree's explore page is its landings", 500, 120);
  is(doc().querySelector("#query")?.value, "landings:7", "the last week's");
  const stacks = [...doc().querySelectorAll(".landing")];
  ok(stacks.length > 0, `It has stacks (${stacks.length})`);
  for (const stack of stacks) {
    const href = stack.querySelector(".landing-link")?.getAttribute("href") ?? "";
    ok(/^\/searchfox\/(explore\/bug\/\d+|commit\/[0-9a-f]{40})$/.test(href),
       `Each links to its review, or its commit (${href})`);
  }
  const parts = doc().querySelector('.landing[data-path="bug/1517978"]');
  ok(parts?.querySelector(".landing-link")?.getAttribute("href") == "/searchfox/explore/bug/1517978",
     "A bug's commits are a stack, linked to its review");
  ok(parts?.querySelector(".landing-meta")?.textContent.includes("commits"), "which says how many commits");
  const landed = stacks.map(stack => stack.querySelector(".landing-meta").textContent.match(/landed (\S+ \S+)/)?.[1] ?? "");
  ok(landed.every((time, i) => i == 0 || landed[i - 1] >= time), "newest first");
  ok(doc().querySelector('.facet[data-facet="author"]') && doc().querySelector('.facet[data-facet="type"]'),
     "Its facets have their authors and files' types");

  await TestUtils.loadPath("/searchfox/explore/?days=30&interesting=1");
  await waitForCondition(() => doc()?.querySelector("#landings"), "The interesting ones", 500, 120);
  is(doc().querySelector("#query")?.value, "interesting-landings:30", "of the last 30 days");
  const scores = [...doc().querySelectorAll(".landing")].map(stack => Number(stack.dataset.size));
  ok(scores.length > 0 && scores.every(score => Number.isFinite(score)), "are weighed");
  ok(scores.every((score, i) => i == 0 || scores[i - 1] >= score), `and sorted by weight (${scores.slice(0, 5)})`);
  ok(doc().querySelector(".landings-about a")?.getAttribute("href") == "/searchfox/explore/?days=30",
     "with a link to them newest first");

  const api = await (await fetch("/searchfox/api/v1/landings?days=1", { cache: "no-store" })).json();
  ok(Array.isArray(api.commits) && api.days == 1, "The data API has the landings too");
  is((await fetch("/searchfox/api/v1/landings?days=0", { cache: "no-store" })).status, 422, "of 1 to 90 days");
});
