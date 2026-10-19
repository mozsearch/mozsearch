"use strict";

// The context menu's "Follow" items (see tools/src/dataflow/), on
// tests/tests/files/dataflow/dataflow.cpp, and the follow-* query terms,
// whose traces are subway maps (see static/js/subway.js), and graphs with
// `follow-format:svg`.

function findFollowItem(menu, text) {
  return [...menu.querySelectorAll(".contextmenu-link")].find(
    link => link.textContent === text);
}

// The trace's subway map, once it's drawn: its model, and its rows (their
// text, and their links' paths and lines, ex: "/tests/source/a.cpp#3").
async function tracedMap() {
  const doc = frame.contentDocument;
  await waitForCondition(() => doc.documentElement.dataset.subwayReady, "The trace's map is drawn");
  const model = frame.contentWindow.SUBWAY;
  const rows = [...doc.querySelectorAll(".sw-card .sw-row:has(.sw-lno)")].map(row => ({
    row,
    text: row.querySelector(".sw-code").textContent,
    href: row.querySelector(".sw-lno").getAttribute("href"),
  }));
  return { doc, model, rows };
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

  const { doc, model, rows } = await tracedMap();
  is(model.trace?.title, "Where value (dataflow.cpp:40) comes from", "The trace has its title");
  ok(doc.querySelector("#sw-legend").textContent.includes(model.trace.title),
     "which the legend shows");
  is(doc.querySelector(".sw-card-seed .sw-name")?.textContent, "UseDefault",
     "Its seed's function's card is marked");
  // The value comes from the `true` passed as GetBoolWithDefault's default.
  const write = rows.find(r => r.text.includes("*aRetVal = aDefault;"));
  ok(write, "The trace goes into the callee that writes the out-parameter");
  ok(write?.row.classList.contains("sw-traced"), "which is a step of the trace");
  ok(write?.href.endsWith("/tests/source/dataflow/dataflow.cpp#26"), "Its rows link to their lines");
  ok(!rows.find(r => r.text.includes("if (rv != 0)")).row.classList.contains("sw-traced"),
     "Rows between steps aren't steps");
  const valueLine = model.lines.find(line => line.names.includes("value"));
  ok(valueLine?.names.includes("aRetVal") && valueLine?.names.includes("aResult"),
     "The value's line goes through the out-parameters (`aResult` too, a hub's name)");
  ok(model.connectors.some(c => c.kind == "call" && c.lines.includes(valueLine.id)),
     "along the calls");

  // The graph, a link away.
  const graph = [...doc.querySelectorAll("#sw-legend a")].find(a => a.textContent == "Dataflow graph");
  ok(graph?.href.includes("follow-format%3Asvg"), "The legend links to the trace's graph");
  const graphLoad = TestUtils.waitForLoad();
  TestUtils.click(graph);
  await graphLoad;
  const svg = frame.contentDocument.querySelector(".query-result svg");
  ok(svg, "The trace's graph is shown");
  ok(svg.textContent.includes("Where value (dataflow.cpp:40) comes from"),
     "The graph has its title");
  ok(svg.textContent.includes("*aRetVal = aDefault;"), "and the callee's write");
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

  // The constructor's initializer, from its parameter, from its callers'
  // literals.
  const { model, rows } = await tracedMap();
  ok(rows.some(r => r.text.includes("mName(aName)")), "The trace finds the field's write");
  const caller = rows.find(r => r.text.includes("\"gadget\""));
  ok(caller?.row.classList.contains("sw-traced"), "The trace finds a caller's argument");
  const ctor = model.cards.find(card => card.func == "Widget::Widget");
  const callerCard = model.cards.find(card => card.func == "MakeWidgetName");
  ok(model.connectors.some(c => c.kind == "call" && c.from == callerCard?.id && c.to == ctor?.id),
     "whose call the value goes along, from the literal");
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

  const { model } = await tracedMap();
  const impl = model.cards.find(card => card.func == "DataflowParams::TestBoolean");
  ok(impl, "The trace reaches the C++ implementation");
  const input = model.lines.find(line => line.names.includes("input"));
  ok(input?.names.includes("a"), "whose parameter is the JS argument's line");
  ok(model.connectors.some(c => c.to == impl?.id && c.lines.includes(input.id)),
     "along the binding's call");
});

// An IPDL struct's field (tests/tests/files/ipdl/PTestDataStructuresCommon.ipdlh),
// from its declaration and from its accessor's call: its values come from the
// struct's constructor's arguments (and its mutable accessor's assignments),
// rather than from the generated C++.
add_task(async function test_DataflowFollowIpdlField() {
  await TestUtils.resetFeatureGate("dataflow");
  const construction = "/tests/source/ipdl/TestDataStructures.cpp#669";
  const traced = async () => {
    const { rows } = await tracedMap();
    return {
      construction: rows.some(r => r.href.endsWith(construction)),
      generated: rows.some(r => r.href.includes("__GENERATED__")),
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
    const { construction: found, generated } = await traced();
    ok(found, `From ${what}, the trace has the struct's construction`);
    ok(!generated, "and not the generated code");
  }
});

// An IPDL message's parameter (tests/tests/files/ipdl/PTestDataStructures.ipdl's
// TestStruct's `i`): forwards, from the `Recv` implementation's parameter;
// backwards, to the `Send` method's callers' arguments, across the message.
add_task(async function test_DataflowFollowIpdlMessageParam() {
  await TestUtils.resetFeatureGate("dataflow");
  for (const [direction, item, line] of [
    ["forward", "Follow where i goes", 260],
    ["backward", "Follow where i comes from", 649],
  ]) {
    await TestUtils.loadPath("/tests/source/ipdl/PTestDataStructures.ipdl");
    const param = [...frame.contentDocument.querySelectorAll("#line-70 span[data-symbols]")]
      .find(span => span.textContent === "i" && span.getAttribute("data-symbols").startsWith("IPC_"));
    ok(param, "The message's parameter's token is there");
    TestUtils.click(param);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, "Context menu is shown for the parameter");
    const follow = findFollowItem(menu, item);
    ok(follow, `The parameter can be followed ${direction}`);

    const loadPromise = TestUtils.waitForLoad();
    TestUtils.click(follow);
    await loadPromise;
    const { model, rows } = await tracedMap();
    ok(rows.some(r => r.href.endsWith(`/tests/source/ipdl/TestDataStructures.cpp#${line}`)),
       `and its trace has TestDataStructures.cpp:${line}`);
    is(model.cards.find(card => card.seed)?.func,
       "mozilla::_ipdltest::TestDataStructuresParent::RecvTestStruct",
       "from the Recv method's parameter");
    ok(model.connectors.some(c => c.kind == "ipc"), "and the message's send to it");
  }
});
