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
    "follow-forward:'S_js_test-ts/ts/src/dataflow.js/viaXpcom().(input)' follow-at:'ts/src/dataflow.js#17'");

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

// A black box's calls (tests/dataflow-mapping.toml's "boxed" group; see
// tools/src/dataflow/mapping.rs), which the trace doesn't step into: one
// collapsed chip on the card calling them, whose menu shows their internals
// (the query again with `follow-internals:`), as the legend's link does; and
// in the dataflow graph (`follow-format:svg`), a node linking to them.
add_task(async function test_DataflowFollowBlackBox() {
  await TestUtils.resetFeatureGate("dataflow");
  await TestUtils.loadPath("/tests/source/dataflow/dataflow.cpp");
  const seed = [...frame.contentDocument.querySelectorAll(`#line-328 span[data-symbols]`)]
    .find(span => span.textContent === "aSeed");
  ok(seed, "BoxedUser's parameter's token is there");
  TestUtils.click(seed);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the parameter");
  let loaded = TestUtils.waitForLoad();
  TestUtils.click(findFollowItem(menu, "Follow where aSeed goes"));
  await loaded;

  let { doc, model } = await tracedMap();
  ok(!model.cards.some(card => card.func == "BoxedWrap" || card.func == "BoxedHelper"),
     "The trace doesn't step into the black box's functions");
  const chip = doc.querySelector(".sw-card .sw-black-box");
  ok(chip?.textContent.includes("Boxed helpers"), `Their calls are a chip on the card: ${chip?.textContent}`);
  ok(chip.title.includes("BoxedWrap, BoxedHelper"), `naming them: ${chip.title}`);
  is(doc.querySelectorAll(".sw-black-box-row").length, 2, "whose rows are marked");
  const chipMenu = doc.querySelector("#context-menu");
  TestUtils.click(chip);
  await waitForShown(chipMenu, "The chip's click shows its menu");
  const show = [...chipMenu.querySelectorAll(".contextmenu-link")]
    .find(link => link.textContent.includes("Show internals of Boxed helpers"));
  ok(show, "which can show their internals");
  ok(new URL(show.href).searchParams.get("q").endsWith(" follow-internals:boxed"),
     `with the query again: ${show.href}`);
  const legendLink = [...doc.querySelectorAll(".sw-legend-black-boxes a")]
    .find(a => a.textContent == "show internals");
  is(legendLink?.getAttribute("href"), show.getAttribute("href"), "as the legend's link does");

  loaded = TestUtils.waitForLoad();
  show.click();
  await loaded;
  ({ model } = await tracedMap());
  ok(model.cards.some(card => card.func == "BoxedWrap") && model.cards.some(card => card.func == "BoxedHelper"),
     "Showing their internals steps into them");
  ok(!model.cards.some(card => card.black_boxes?.length), "and collapses nothing");

  // (The dataflow graph's collapsed node links to the internals too.)
  const query = frame.contentDocument.querySelector("#query").value
    .replace(" follow-internals:boxed", "");
  await TestUtils.loadQuery("tests", `${query} follow-format:svg`);
  const node = [...frame.contentDocument.querySelectorAll("svg a")]
    .find(a => (a.getAttribute("href") ?? a.getAttribute("xlink:href")) == "#follow-internals:boxed");
  ok(node, "The graph's black box node links to its internals");
  loaded = TestUtils.waitForLoad();
  TestUtils.click(node, { bubbles: true, cancelable: true });
  await loaded;
  const q = new URL(frame.contentWindow.location.href).searchParams.get("q");
  ok(q.includes("follow-format:svg") && q.endsWith(" follow-internals:boxed"),
     `which loads the graph of the trace stepping into them: ${q}`);
});

// A macro with a signature (tests/dataflow-mapping.toml's DF_TRY_ASSIGN; see
// tools/src/dataflow/mapping.rs) is a block of its labeled arguments, all of
// its lines (here, two), whose traced arguments are highlighted.
add_task(async function test_DataflowFollowMacroComposite() {
  await TestUtils.resetFeatureGate("dataflow");
  await TestUtils.loadPath("/tests/source/dataflow/dataflow.cpp");
  const input = [...frame.contentDocument.querySelectorAll(`#line-347 span[data-symbols]`)]
    .find(span => span.textContent === "aInput");
  ok(input, "ConsumeResult's parameter's token is there");
  TestUtils.click(input);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the parameter");
  const loaded = TestUtils.waitForLoad();
  TestUtils.click(findFollowItem(menu, "Follow where aInput goes"));
  await loaded;

  const { doc } = await tracedMap();
  const head = doc.querySelector(".sw-composite-head");
  is(head?.querySelector(".sw-composite-name")?.textContent, "DF_TRY_ASSIGN",
     "The macro's invocation is a block, named after it");
  is(head.querySelector(".sw-composite-name").title, "DF_TRY_ASSIGN(target, expr)",
     "whose signature is its name's tooltip");
  const args = [...doc.querySelectorAll(".sw-composite-arg")].map(row => ({
    label: row.querySelector(".sw-composite-label").textContent,
    code: row.querySelector(".sw-code").textContent.replace(row.querySelector(".sw-composite-label").textContent, "").trim(),
    traced: row.classList.contains("sw-traced"),
  }));
  is(JSON.stringify(args.map(arg => [arg.label, arg.code])),
     JSON.stringify([["into", "value"], ["tries", "ProduceResult(aInput)"]]),
     `with its arguments, labeled, from both of its lines: ${JSON.stringify(args)}`);
  ok(args.every(arg => arg.traced), "both traced, as the value goes into the target");
});

// Each line is one through the cards and between them (see subway.js's
// `afterLayout`): its edges end at its tracks, at their x, in the cards at
// their ends: at a card's top, whose head has the track, or at its bottom,
// whose last row's has it, down to the bottom.  ConsumeResult's call of
// ProduceResult goes down to it, and its return back up.
add_task(async function test_DataflowFollowContinuousLines() {
  await TestUtils.resetFeatureGate("dataflow");
  await TestUtils.loadPath("/tests/source/dataflow/dataflow.cpp");
  const input = [...frame.contentDocument.querySelectorAll(`#line-347 span[data-symbols]`)]
    .find(span => span.textContent === "aInput");
  TestUtils.click(input);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the parameter");
  const loaded = TestUtils.waitForLoad();
  TestUtils.click(findFollowItem(menu, "Follow where aInput goes"));
  await loaded;

  const { doc } = await tracedMap();
  const container = doc.querySelector("#interactive-graph-container");
  const cards = [...container.querySelectorAll(".sw-card")].map(card => ({
    card,
    left: parseFloat(card.style.left),
    top: parseFloat(card.style.top),
    width: card.offsetWidth,
    height: card.offsetHeight,
  }));
  const ends = [];
  for (const path of container.querySelectorAll("path.interactive-graph-edge:not(.clicktarget)")) {
    const line = /interactive-graph-edge-line-(\d+)/.exec(path.getAttribute("class"))?.[1];
    if (line == null) {
      continue;
    }
    for (const at of [0, path.getTotalLength()]) {
      const { x, y } = path.getPointAtLength(at);
      const end = { line, x: Math.round(x), y: Math.round(y) };
      for (const c of cards) {
        if (x < c.left || x > c.left + c.width) {
          continue;
        }
        // (1px in, over its border.)
        const side = Math.abs(y - (c.top + 1)) <= 1 ? "top"
          : Math.abs(y - (c.top + c.height - 1)) <= 1 ? "bottom" : null;
        if (!side) {
          continue;
        }
        end.card = c.card.querySelector(".sw-name").textContent;
        end.side = side;
        const track = side == "top"
          ? c.card.querySelector(`.sw-head-tracks line[data-line="${line}"]`)
          : [...c.card.querySelectorAll(".sw-row")].at(-1)
            .querySelector(`.sw-gutter line[data-line="${line}"][y2="10"]`);
        end.track = !!track;
        end.trackX = track ? c.left + c.card.clientLeft + Number(track.getAttribute("x1")) : null;
      }
      ends.push(end);
    }
  }
  ok(ends.length >= 4, `The lines have edges: ${JSON.stringify(ends)}`);
  ok(ends.every(end => end.track && Math.abs(end.trackX - end.x) <= 1),
     `which end at their tracks: ${JSON.stringify(ends)}`);
  ok(ends.some(end => end.side == "top") && ends.some(end => end.side == "bottom"),
     "at cards' tops and bottoms");
});
