"use strict";
// webtest-group: trydelta-push (they import it)

// A try push's call roll-ups (see build-try-delta and `RollupDelta`): the
// trydelta tree's push abcdef012345 adds meter.cpp, whose Meter class calls
// Widget's Shrink and Size, so in the push's context Widget's and Meter's
// roll-ups have an edge between them, which the indexed revision's don't.

const PUSH = "abcdef012345";
const EDGE = "T_trydelta::Meter->T_trydelta::Widget";

function doc() {
  return frame.contentDocument;
}

// Import the push (if it isn't yet), and return the head of its stack rebased
// onto the indexed revision (as test_TryDelta.js's `importPush` does).
async function importPush() {
  await TestUtils.loadPath(`/trydelta/try/${PUSH}`);
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  const link = doc().querySelector(".explore-semantic-head a")?.getAttribute("href");
  ok(link?.startsWith("/trydelta/rev/"), "The push's page links to its semantic analysis");
  return link.split("/")[3];
}

function diagramEdges() {
  return [...doc().querySelectorAll("svg [data-symbols]")]
    .map(e => e.getAttribute("data-symbols"));
}

add_task(async function test_TryRollups() {
  await TestUtils.resetFeatureGate("diagramming");
  const head = await importPush();

  for (const cls of ["T_trydelta::Widget", "T_trydelta::Meter"]) {
    await TestUtils.loadPath(
      `/trydelta/query/default?q=${encodeURIComponent(`block-diagram:'${cls}'`)}&rev=${head}`);
    ok(diagramEdges().includes(EDGE), `${cls}'s block diagram has the push's edge`);
    const texts = [...doc().querySelectorAll("svg text")].map(t => t.textContent);
    ok(texts.includes("2"), `which is labeled with its calls: ${texts.join(", ")}`);
  }

  await TestUtils.loadQuery("trydelta", "block-diagram:'T_trydelta::Widget'");
  ok(!diagramEdges().includes(EDGE), "The indexed revision's Widget doesn't have it");
});

// The context menu's diagrams of the push's code are in its context.
add_task(async function test_TryRollupsFromClass() {
  await TestUtils.resetFeatureGate("diagramming");
  const head = await importPush();

  await TestUtils.loadPath(`/trydelta/rev/${head}/meter.cpp`);
  const meter = doc().querySelector(`span[data-symbols="T_trydelta::Meter"]`);
  ok(meter, "The push's class's token is there");
  TestUtils.click(meter);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the class");

  const title = menu.querySelector(".contextmenu-section-title.icon-brush");
  const block = [...(title?.closest("li").querySelectorAll(".contextmenu-button") ?? [])]
    .find(button => button.textContent === "Block");
  ok(block?.getAttribute("href")?.endsWith(`&rev=${head}`),
     "The class's block diagram is in the push's context");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(block);
  await loadPromise;
  ok(diagramEdges().includes(EDGE), "and has the push's edge");
});
