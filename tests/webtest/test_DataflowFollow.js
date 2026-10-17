"use strict";

// The context menu's "Follow" items (see tools/src/dataflow/), on
// tests/tests/files/dataflow/dataflow.cpp, and the follow-* query terms.

function findFollowItem(menu, text) {
  return [...menu.querySelectorAll(".contextmenu-link")].find(
    link => link.textContent === text);
}

add_task(async function test_DataflowFollowLocal() {
  await TestUtils.resetFeatureGate("dataflow");
  await TestUtils.loadPath("/tests/source/dataflow/dataflow.cpp");

  // `return value;` in UseDefault.
  const value = [...frame.contentDocument.querySelectorAll(`#line-40 span[data-symbols]`)]
    .find(span => span.textContent === "value");
  ok(value, "The local's token is there");
  TestUtils.click(value);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the local");

  ok(findFollowItem(menu, "Follow where value goes"), "Forward item is shown");
  const backward = findFollowItem(menu, "Follow where value comes from");
  ok(backward, "Backward item is shown");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(backward);
  await loadPromise;

  const query = frame.contentDocument.querySelector("#query").value;
  ok(query.startsWith("follow-backward:") && query.endsWith("follow-at:dataflow/dataflow.cpp#40"),
     `The query follows the local backward from its line: ${query}`);

  const svg = frame.contentDocument.querySelector(".query-result svg");
  ok(svg, "The trace is shown");
  ok(svg.textContent.includes("Where value (dataflow.cpp:40) comes from"),
     "The trace has its title");
  // The value comes from the `true` passed as GetBoolWithDefault's default.
  ok(svg.textContent.includes("*aRetVal = aDefault;"),
     "The trace goes into the callee that writes the out-parameter");
  const link = [...svg.querySelectorAll("a")].find(
    a => a.getAttribute("xlink:href")?.endsWith("/tests/source/dataflow/dataflow.cpp#26"));
  ok(link, "Its nodes link to their lines");
});

add_task(async function test_DataflowFollowField() {
  await TestUtils.resetFeatureGate("dataflow");
  await TestUtils.loadPath("/tests/source/dataflow/dataflow.cpp");

  // `const char* Name() const { return mName; }`
  const field = [...frame.contentDocument.querySelectorAll(`#line-46 span[data-symbols]`)]
    .find(span => span.textContent === "mName");
  ok(field, "The field's token is there");
  TestUtils.click(field);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the field");

  const backward = findFollowItem(menu, "Follow where mName comes from");
  ok(backward, "Backward item is shown");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(backward);
  await loadPromise;

  const svg = frame.contentDocument.querySelector(".query-result svg");
  ok(svg, "The trace is shown");
  // The constructor's initializer, from its parameter, from its callers'
  // literals.
  ok(svg.textContent.includes("mName(aName)"), "The trace finds the field's write");
  ok(svg.textContent.includes("\"gadget\""), "The trace finds a caller's argument");
});

add_task(async function test_DataflowFeatureGate() {
  await TestUtils.setFeatureGate("dataflow", "release");
  await TestUtils.loadPath("/tests/source/dataflow/dataflow.cpp");

  const value = [...frame.contentDocument.querySelectorAll(`#line-40 span[data-symbols]`)]
    .find(span => span.textContent === "value");
  TestUtils.click(value);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the local");
  ok(!findFollowItem(menu, "Follow where value goes"),
     "Forward item isn't shown without the feature");

  await TestUtils.resetFeatureGate("dataflow");
});

add_task(async function test_DataflowQueryJs() {
  await TestUtils.resetFeatureGate("dataflow");
  // From JS's `input` through XPIDL's binding into the C++ implementation.
  await TestUtils.loadQuery(
    "tests",
    "follow-forward:'S_js_ts/src/dataflow.js/viaXpcom().(input)' follow-at:'ts/src/dataflow.js#17'");

  const svg = frame.contentDocument.querySelector(".query-result svg");
  ok(svg, "The trace is shown");
  ok(svg.textContent.includes("param a"),
     "The trace reaches the C++ implementation's parameter");
});

// An IPDL struct's field (tests/tests/files/ipdl/PTestDataStructuresCommon.ipdlh),
// from its declaration and from its accessor's call: its values come from the
// struct's constructor's arguments (and its mutable accessor's assignments),
// rather than from the generated C++.
add_task(async function test_DataflowFollowIpdlField() {
  await TestUtils.resetFeatureGate("dataflow");
  const construction = "/tests/source/ipdl/TestDataStructures.cpp#669";
  const traced = () => {
    const svg = frame.contentDocument.querySelector(".query-result svg");
    const links = [...(svg?.querySelectorAll("a") ?? [])].map(a => a.getAttribute("xlink:href"));
    return {
      construction: links.some(href => href?.endsWith(construction)),
      generated: links.some(href => href?.includes("__GENERATED__")),
    };
  };

  for (const [path, line, isToken, what] of [
    ["/tests/source/ipdl/PTestDataStructuresCommon.ipdlh", 27,
     span => span.textContent === "ai", "the field's declaration"],
    ["/tests/source/ipdl/TestDataStructures.cpp", 277,
     span => span.textContent === "ai" && span.getAttribute("data-symbols").startsWith("_ZNK"),
     "the field's accessor's call"],
  ]) {
    await TestUtils.loadPath(path);
    const token = [...frame.contentDocument.querySelectorAll(`#line-${line} span[data-symbols]`)]
      .find(isToken);
    ok(token, `The token of ${what} is there`);
    TestUtils.click(token);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, `Context menu is shown for ${what}`);
    const backward = findFollowItem(menu, "Follow where ai comes from");
    ok(backward, `${what} can be followed`);

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(backward);
    await loadPromise;
    const { construction: found, generated } = traced();
    ok(found, `From ${what}, the trace has the struct's construction`);
    ok(!generated, "and not the generated code");
  }
});
