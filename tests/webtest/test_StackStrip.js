"use strict";
// webtest-group: trydelta-push (they import it)

// The patch stack strip (see tools/src/hyperblame/stack_strip.rs and
// static/js/stack-strip.js): files' views opened from explore pages (and try
// pushes' pages) show beside the blame strip which of the stack's commits
// changed each line.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_StackStripExplore() {
  // Bug 1975601's Part 1 added find_phab_rev to blame.rs.
  await TestUtils.loadPath("/searchfox/explore/bug/1975601");
  const link = [...doc().querySelectorAll(".explore-file-header a")].find(
    a => a.textContent == "tools/src/blame.rs");
  ok(link?.getAttribute("href").endsWith("?stack=explore/bug/1975601"),
     "An explore page's files link to their views with its stack");

  await TestUtils.loadPath(link.getAttribute("href"));
  await waitForCondition(() => doc().querySelector(".stack-strip"),
                         "which show the stack strip");
  const row = [...doc().querySelectorAll(".source-line-with-number")].find(
    r => r.querySelector("code")?.textContent.includes("fn find_phab_rev"));
  ok(row?.querySelector('.stack-slot[data-slot="0"]'),
     "Part 1 changed the line of the function it added");
  is(doc().querySelector(".stack-strip-note a")?.getAttribute("href"),
     "/searchfox/explore/bug/1975601", "A note links back to the explore page");
  // (And the panel's Changes section goes to the stack's changes; see
  // test_ChangeNav.js.)
  await waitForCondition(() => doc().querySelector(".panel .change-nav"),
                         "The panel can go to the stack's changes");
  TestUtils.click(doc().querySelector('.change-nav button[data-accel="J"]'));
  ok(doc().querySelector(".change-nav-current .stack-slot") ||
     [...doc().querySelectorAll(".change-nav-current")].length > 0,
     "and marks the rows of the one it goes to");
});

add_task(async function test_StackStripTry() {
  // The spechist tree's try push fedcba987654 renames Poke to Prod and removes
  // Never, and then renames Prod to Prodded (see tests/spechist/try).
  const push = "fedcba987654";
  await fetch(`/spechist/try/hg/${push}`, { method: "POST" });
  let status;
  for (let i = 0; i < 240; i++) {
    status = await (await fetch(`/spechist/try-status/hg/${push}`)).json();
    if (status.state == "imported" || status.state == "failed") {
      break;
    }
    await TestUtils.sleep(500);
  }
  is(status.state, "imported", "The try push is imported");

  await TestUtils.loadPath(`/spechist/try/hg/${push}`);
  const link = doc().querySelector('.explore-file-header a[href*="cite.cpp"]');
  ok(link?.getAttribute("href").endsWith(`?stack=try/hg/${push}`),
     "Its page's files link to their views with the push");
  await TestUtils.loadPath(link.getAttribute("href"));
  await waitForCondition(() => doc().querySelector(".stack-strip"),
                         "which show the stack strip");
  const slots = n => [...doc().querySelectorAll(`#line-${n} .stack-slot:not(.stack-scar)`)]
    .map(slot => slot.dataset.slot);
  is(slots(9).join(","), "0,1", "Both commits changed Prodded's line, the first as Prod");
  ok(doc().querySelector('#line-12 .stack-scar[data-slot="0"]'),
     "The first's removal of Never is a scar on the line before it");
  is(doc().querySelectorAll("#line-6 .stack-slot").length, 0,
     "Lines the push didn't change have no slots");

  // Its page's other links pass the push on as they're followed (ex: its
  // outline rows' links to the definitions it changed; see stack-strip.js),
  // and in its context the outline's changes are since its base (see
  // outline.js), rather than since a release.
  await TestUtils.loadPath(`/spechist/try/hg/${push}`);
  const row = doc().querySelector(".explore-outline-name a");
  ok(row && !row.getAttribute("href").includes("stack="),
     "Its outline rows link to the definitions it changed");
  row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  ok(row.getAttribute("href").includes(`?stack=try/hg/${push}`), "with the push, once followed");
  await TestUtils.loadPath(row.getAttribute("href"));
  await waitForCondition(() => doc().querySelector(".stack-strip"),
                         "whose views show the stack strip");
  await waitForCondition(
    () => doc().querySelector(".outline-baseline button")?.textContent == "the try push's base",
    "and the outline's changes since the push's base");
});
