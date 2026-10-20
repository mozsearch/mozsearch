"use strict";

// A tree's landing page (`/{tree}/`; see `format::format_landing`): a link to
// browse its source, its system diagrams (block diagrams of its subsystems,
// from its roll-ups), and its recently landed bugs, linking to their explore
// pages.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_LandingDiagrams() {
  // (Before loading the page: it loads the settings page.)
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadPath("/tests/");
  is(doc().querySelector(".landing-browse-link")?.getAttribute("href"), "/tests/source/",
     "It links to the tree's source");

  const query = link => new URL(link.href).searchParams.get("q");
  const [calls, data] = [...doc().querySelectorAll(".landing-diagram")];
  is(calls && query(calls), "block-diagram:*", "and to the diagram of all of its subsystems");
  is(data && query(data), "block-diagram:* block-edges:data", "by calls and by data");
  is(query(doc().querySelector(".landing-arcs")), "block-diagram:* block-view:arcs",
     "and in a column with arcs");
  const subsystem = [...doc().querySelectorAll(".landing-subsystems a")].find(
    a => a.textContent == "Core/Rollup View");
  is(subsystem && query(subsystem), "block-diagram:'Core/Rollup View'",
     "and to each subsystem's diagram");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(subsystem);
  await loadPromise;
  const texts = [...doc().querySelectorAll("svg text")].map(t => t.textContent);
  ok(texts.includes("Panel"), `which shows its classes: ${texts.join(", ")}`);
});

add_task(async function test_LandingBugs() {
  // (The searchfox tree is this checkout, with its history and commit index.)
  await TestUtils.loadPath("/searchfox/");
  const bugs = [...doc().querySelectorAll(".landing-bug-list li")];
  ok(bugs.length > 0, "It lists the bugs which landed most recently");
  const bug = bugs[0].querySelector(".landing-bug").textContent.replace("Bug ", "");
  is(bugs[0].querySelector(".landing-explore")?.getAttribute("href"),
     `/searchfox/explore/bug/${bug}`, "each linking to its explore page");
  ok(bugs[0].querySelector(".landing-bug").href.endsWith(`show_bug.cgi?id=${bug}`),
     "and to the bug");

  // (A tree without explore pages lists none.)
  await TestUtils.loadPath("/tests/");
  ok(!doc().querySelector(".landing-bugs"), "The tests tree has no recently landed bugs");
});
