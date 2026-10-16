"use strict";

// The fancy bar's "Inside" section says what the hovered line is inside, from
// the source listing's nesting containers (see panel.js's
// `addInsideSection`), ex: in a taskgraph kind, where in a task it is.
add_task(async function test_InsideSection() {
  await TestUtils.resetFeatureGate("fancyBar");
  await TestUtils.loadPath("/tests/source/taskcluster/kinds/package/kind.yml");
  const doc = frame.contentDocument;
  const section = doc.getElementById("panel-inside");
  ok(section, "The fancy bar has the Inside section");

  TestUtils.dispatchMouseEvent("mouseover", doc.querySelector("#line-22 code"));
  is(section.textContent, "tasks › linux64/opt › fetches › toolchain",
     "A hovered line says what it's inside, outermost first");
  const fetches = [...section.querySelectorAll("a")].find(link => link.textContent == "fetches");
  is(fetches?.getAttribute("href"), "#20", "each linking to its line");

  TestUtils.dispatchMouseEvent("mouseover", doc.querySelector("#line-21 code"));
  is(section.textContent, "tasks › linux64/opt › fetches",
     "A line starting a nesting container isn't inside it");

  TestUtils.dispatchMouseEvent("mouseover", doc.querySelector("#line-2 code"));
  is(section.textContent, "(the top level)", "and a top-level line is at the top level");
});
