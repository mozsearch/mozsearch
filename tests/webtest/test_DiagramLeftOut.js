"use strict";

// The calls a calls diagram leaves out (see config_defaults/diagram-mapping.toml,
// and tests/tests/files/cpp/left_out_calls.cpp's imitations of Gecko's
// infrastructure), in the navigation panel's legend, whose buttons show them
// (`show-calls:`) and leave them out again.

const QUERY = "calls-from:'left_out_calls::Works' depth:2";

function diagramText() {
  return [...frame.contentDocument.querySelectorAll("svg text")]
    .map(text => text.textContent)
    .join(" ");
}

// A group's row in the legend: its name, its count, and its button.
function leftOutGroup(name) {
  const legend = frame.contentDocument.querySelector(
    "#panel-content #diagram-legend-pane .diagram-left-out");
  ok(legend, "The legend has the left-out calls");
  const row = [...legend.querySelectorAll("li")]
    .find(li => li.querySelector(".diagram-left-out-name").textContent == name);
  ok(row, `with the group ${name}`);
  return {
    title: row.querySelector(".diagram-left-out-name").title,
    count: row.querySelector(".diagram-left-out-count").textContent,
    button: row.querySelector("button"),
  };
}

add_task(async function test_DiagramLeftOut_legend() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", QUERY);

  ok(diagramText().includes("Poke"), "The diagram has the code's own calls");
  ok(!diagramText().includes("MOZ_ReportAssertionFailure"),
     "but not the assertions' calls");
  ok(!diagramText().includes("Value"), "nor the getter's");
  ok(!diagramText().includes("isSome"), "nor Maybe's");

  const assertions = leftOutGroup("Assertions and crashes");
  ok(assertions.title.includes("check the code's logic"),
     "whose tooltip says why");
  ok(assertions.title.includes("AnnotateMozCrashReason, MOZ_ReportAssertionFailure"),
     "and the functions called");
  is(assertions.count, "(2)", "It says how many functions are left out");
  is(assertions.button.textContent, "Show", "and can show them");
  leftOutGroup("Getters");
  leftOutGroup("Strings");

  // A group's button shows its calls.
  let loadPromise = TestUtils.waitForLoad();
  TestUtils.click(assertions.button);
  await loadPromise;

  is(frame.contentDocument.querySelector("#query").value,
     `${QUERY} show-calls:assertions`,
     "The query shows the group's calls");
  ok(diagramText().includes("MOZ_ReportAssertionFailure"),
     "The diagram has the assertions' calls");
  ok(!diagramText().includes("Value"), "but not the getter's");

  // And leaves them out again.
  const shown = leftOutGroup("Assertions and crashes");
  is(shown.count, "(2, shown)", "It says they're shown");
  is(shown.button.textContent, "Leave out", "and can leave them out");
  loadPromise = TestUtils.waitForLoad();
  TestUtils.click(shown.button);
  await loadPromise;

  is(frame.contentDocument.querySelector("#query").value, QUERY,
     "The query leaves them out again");
});

add_task(async function test_DiagramLeftOut_all() {
  await TestUtils.loadQuery("tests", QUERY);

  const button = [...frame.contentDocument.querySelectorAll(
    "#diagram-legend-pane button.diagram-show-calls")]
    .find(b => b.textContent == "Show them all");
  ok(button, "The legend can show all the left-out calls");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(button);
  await loadPromise;

  const query = frame.contentDocument.querySelector("#query").value;
  ok(query.startsWith(`${QUERY} show-calls:`) && query.includes("getters"),
     "The query shows all the groups the diagram left out");
  ok(diagramText().includes("Value") && diagramText().includes("isSome"),
     "The diagram has their calls");
});

add_task(async function test_DiagramLeftOut_ofGroup() {
  // A diagram of a group's function shows that group's calls.
  await TestUtils.loadQuery("tests", "calls-from:'mozilla::UniquePtr::reset' depth:2");

  ok(diagramText().includes("get"), "The diagram has the smart pointer's calls");
  is(leftOutGroup("Smart pointers and reference counting").count,
     "(1, shown: the diagram's of them)", "which the legend says it shows");
});
