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
});

add_task(async function test_StackStripTry() {
  // The spechist tree's try push fedcba987654 renames Poke to Prod and removes
  // Never, and then renames Prod to Prodded (see tests/spechist/try).
  const push = "fedcba987654";
  await fetch(`/spechist/try/${push}`, { method: "POST" });
  let status;
  for (let i = 0; i < 240; i++) {
    status = await (await fetch(`/spechist/try-status/${push}`)).json();
    if (status.state == "imported" || status.state == "failed") {
      break;
    }
    await TestUtils.sleep(500);
  }
  is(status.state, "imported", "The try push is imported");

  await TestUtils.loadPath(`/spechist/try/${push}`);
  const link = doc().querySelector('.explore-file-header a[href*="cite.cpp"]');
  ok(link?.getAttribute("href").endsWith(`?stack=try/${push}`),
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
});
