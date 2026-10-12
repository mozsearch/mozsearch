"use strict";

// Citations of anchors their spec no longer has (see
// tools/src/spec_analyze/stale.rs), in the spechist tree, whose spec's history
// renames and removes definitions which cite.cpp cites (see
// tests/spechist/setup, whose forged dates give its commits these SHAs).
const RENAMING_COMMIT = "211207bded07dc0461abbbeb72adf41d0c087661";
const REMOVING_COMMIT = "e133e2c5ba88929344851cb60421c1cf54adbce8";

async function citationMenuLinks(line, sym, { stale = true } = {}) {
  await TestUtils.loadPath("/spechist/source/cite.cpp");
  const span = frame.contentDocument.querySelector(
    `#line-${line} span[data-symbols="${sym}"]`);
  ok(span, `The citation on line ${line} has its symbol`);
  is(span.classList.contains("syn_stale"), stale,
     stale ? "and is marked as stale" : "and isn't marked as stale");

  TestUtils.click(span);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the citation");
  return [...menu.querySelectorAll("a")];
}

// The renamed definition's text is in the definition at the tip which it may
// be now.
add_task(async function test_renamed_definition() {
  const links = await citationMenuLinks(5, "SPEC_hist_concept@2Dwidget@2Dfrob");
  const removed = links.find(a => a.textContent.includes("is no longer in its spec"));
  ok(removed?.textContent.includes("hist #concept-widget-frob"),
     "It says which anchor is no longer in its spec");
  is(removed?.getAttribute("href"), `/spechist/commit/${RENAMING_COMMIT}`,
     "and which commit removed it");
  const maybe = links.filter(a => a.textContent.includes("Maybe it's now"));
  is(maybe.length, 1, "It has a suggestion");
  ok(maybe[0].textContent.includes("hist widget frobbing"),
     "of the renamed definition");
  is(maybe[0].getAttribute("href"), "/spechist/source/hist/source#8",
     "going to its definition");
});

// The removed definition has no suggestions.
add_task(async function test_removed_definition() {
  const links = await citationMenuLinks(11, "SPEC_hist_concept@2Dgadget@2Dhum");
  const removed = links.find(a => a.textContent.includes("is no longer in its spec"));
  is(removed?.getAttribute("href"), `/spechist/commit/${REMOVING_COMMIT}`,
     "It says which commit removed it");
  ok(!links.some(a => a.textContent.includes("Maybe it's now")),
     "and suggests nothing");
});

// A citation of a current definition isn't stale, nor is one of an anchor the
// spec never had.
add_task(async function test_not_stale() {
  const links = await citationMenuLinks(8, "SPEC_hist_concept@2Dwidget@2Dpoke", { stale: false });
  ok(links.find(a => a.textContent.includes("Go to definition of")),
     "A current definition's citation goes to it");
  ok(!links.some(a => a.textContent.includes("is no longer in its spec")),
     "and isn't stale");

  await TestUtils.loadPath("/spechist/source/cite.cpp");
  const never = frame.contentDocument.querySelector(
    `#line-14 span[data-symbols="SPEC_hist_concept@2Dnever@2Ddefined"]`);
  ok(never && !never.classList.contains("syn_stale"),
     "A citation of an anchor the spec never had isn't stale");
});
