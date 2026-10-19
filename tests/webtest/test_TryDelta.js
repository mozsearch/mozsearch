"use strict";
// webtest-group: trydelta-push (they import it)

// The trydelta tree's try pushes get semantic analysis (see
// tests/trydelta/analyze-try and tools/src/try_delta.rs): 0123456789ab changes
// Widget::Frob's signature (which user.cpp's call and gadget.h's then use,
// though they don't change), adds Widget::Wobble, and adds wobbler.cpp, which
// calls it.  It also renames hello.js's greetPerson, which its analysis (only
// of the tree's C++) doesn't cover, and changes unpoke.cpp's implementation
// of the tests tree's example spec's unpoke algorithm (the tree's `SPEC_`
// symbols are the tests tree's), whose step it cites.  (Not the poke
// algorithm, whose citations in the searchfox tree, which has this file too,
// test_SpecCitations.js's diagram has as many of as it shows.)

const PUSH = "0123456789ab";
const NEW_FROB = "_ZN8trydelta6Widget4FrobEib";
const UNPOKE = "SPEC_example_unpoke";
const OLD_FROB = "_ZN8trydelta6Widget4FrobEi";

function doc() {
  return frame.contentDocument;
}

// Import the push (if it isn't yet), and return the head of its stack rebased
// onto the indexed revision, from its page's link to its semantic analysis.
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

add_task(async function test_TryDeltaPages() {
  const head = await importPush();
  is(doc().querySelector("#data").getAttribute("data-search-rev"), head,
     "and searches from it are of the push");

  await TestUtils.loadPath(`/trydelta/rev/${head}/user.cpp`);
  is(doc().querySelector("#data").getAttribute("data-search-rev"), head,
     "The head's pages search in the push's context");
  is(doc().querySelector("#search-box").getAttribute("action"), "/trydelta/search",
     "with /search");
  is(doc().querySelector('#search-box input[name="rev"]')?.value, head, "for the push");
  // (Which queries in the push's context.)
  doc().querySelector("#query").value = "Frob";
  const loaded = TestUtils.waitForLoad();
  doc().querySelector("#search-box").requestSubmit();
  await loaded;
  is(doc().location.pathname, "/trydelta/query/default", "Searching from them queries");
  is(new URLSearchParams(doc().location.search).get("rev"), head, "in the push's context");
  await TestUtils.loadPath(`/trydelta/rev/${head}/user.cpp`);

  const frob = [...doc().querySelectorAll("#line-8 .source-line span[data-symbols]")]
    .find(span => span.textContent == "Frob");
  ok(frob?.getAttribute("data-symbols").split(",").includes(NEW_FROB),
     "An unchanged file's call has the symbol of the signature the push gives it");
  ok(frame.contentWindow.SYM_INFO[NEW_FROB]?.jumps?.def?.startsWith("widget.cpp"),
     "which jumps to the push's definition");

  TestUtils.click(frob);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");
  const search = menu.querySelector(".icon-search")?.getAttribute("href");
  ok(search?.startsWith("/trydelta/query/default?q=symbol:") && search.endsWith(`&rev=${head}`),
     "and its searches are of the push");
  const gotoDef = [...menu.querySelectorAll("a")]
    .find(a => a.textContent.includes("Go to definition"));
  ok(gotoDef?.getAttribute("href")?.startsWith(`/trydelta/rev/${head}/widget.cpp#`),
     "and its jumps are to the push's head's files");
});

add_task(async function test_TryDeltaQueries() {
  const head = await importPush();

  await TestUtils.loadPath(`/trydelta/query/default?q=Wobble&rev=${head}`);
  ok(doc().querySelector(".query-try-context")?.textContent.includes(PUSH),
     "A query in the push's context says so");
  ok(doc().querySelector(`a[href="/trydelta/rev/${head}/wobbler.cpp"]`),
     "and finds the files the push adds, at its head");
  ok(doc().querySelector(`a.query-line-number[href^="/trydelta/rev/${head}/"]`),
     "whose lines link to its head's");
  is(doc().querySelector('#search-box input[name="rev"]')?.value, head,
     "Searching again stays in the push's context");
  await TestUtils.loadPath("/trydelta/query/default?q=Wobble");
  ok(!doc().querySelector('a[href$="/wobbler.cpp"]'), "The indexed revision doesn't have them");

  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${NEW_FROB}&rev=${head}`);
  ok(doc().querySelector('a[href$="/user.cpp"]'),
     "The push's new symbols' uses include those of files it doesn't change");
  ok(doc().querySelector('a[href$="/gadget.h"]'), "headers included");
  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${OLD_FROB}&rev=${head}`);
  ok(!doc().querySelector('a[href$="/user.cpp"]'), "and the symbols it replaces have none");
  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${OLD_FROB}`);
  ok(doc().querySelector('a[href$="/user.cpp"]'), "unlike in the indexed revision");
});

// Following a value in the push's code (see tools/src/dataflow/) follows it
// through the push's: the parameter it adds to Widget::Frob.
add_task(async function test_TryDeltaFollow() {
  await TestUtils.resetFeatureGate("dataflow");
  const head = await importPush();

  await TestUtils.loadPath(`/trydelta/rev/${head}/widget.cpp`);
  // `mSize += aLoudly ? aAmount * 2 : aAmount;`
  const param = [...doc().querySelectorAll("#line-6 span[data-symbols]")]
    .find(span => span.textContent === "aLoudly");
  ok(param, "The push's parameter's token is there");
  TestUtils.click(param);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the parameter");

  const backward = [...menu.querySelectorAll(".contextmenu-link")]
    .find(link => link.textContent === "Follow where aLoudly comes from");
  ok(backward?.getAttribute("href")?.endsWith(`&rev=${head}`),
     "Following it is in the push's context");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(backward);
  await loadPromise;
  // (Its subway map; see test_DataflowFollow.)
  await waitForCondition(() => doc().documentElement.dataset.subwayReady, "The trace's map is drawn");
  is(frame.contentWindow.SUBWAY.trace?.title, "Where aLoudly (widget.cpp:6) comes from",
     "The trace is of the push's code");
  ok([...doc().querySelectorAll(".sw-traced .sw-lno")].some(
    a => a.getAttribute("href") === `/trydelta/rev/${head}/widget.cpp#6`),
     "and its steps' rows link to the push's head's lines");
});

// Slicing a revision's changes into the clusters of dataflow they touch (see
// tools/src/dataflow/slice.rs and rev_lines.rs): the push's, in its context,
// and a landed commit's, in the indexed revision.
add_task(async function test_TryDeltaSlicing() {
  const head = await importPush();
  const sliceLinks = () => [...(doc().querySelector(".query-result svg")?.querySelectorAll("a") ?? [])]
    .map(a => a.getAttribute("xlink:href"));

  // (The dataflow graph, rather than the subway map; see test_TryDeltaSubway.)
  await TestUtils.loadPath(
    `/trydelta/query/default?q=${encodeURIComponent(`slice-rev:${head} slice-format:svg`)}&rev=${head}`);
  ok(sliceLinks().some(href => href?.startsWith(`/trydelta/rev/${head}/wobbler.cpp#`)),
     "The push's slicing's units link to its head's lines, of the files it adds too");
  ok(doc().querySelector(".query-result svg").textContent.includes("cluster 2"),
     "in several clusters (Frob's signature, Wobble, ...)");

  await TestUtils.loadPath(
    `/trydelta/query/default?q=${encodeURIComponent(`slice-rev:${head} slice-cluster:1 slice-format:svg`)}&rev=${head}`);
  const one = doc().querySelector(".query-result svg")?.textContent ?? "";
  ok(one.includes("cluster 1") && !one.includes("cluster 2"), "One of them can be shown");

  // (What it removes, from the indexed revision's analysis: the old Frob.)
  const response = await fetch(
    `/trydelta/query/default?q=${encodeURIComponent(`slice-rev:${head} slice-format:json`)}&rev=${head}`,
    { headers: { Accept: "application/json" }, cache: "no-store" });
  const slicing = (await response.json())?.JsonValue?.value;
  const clusterOf = unit => slicing.clusters.findIndex(c => c.includes(unit.id));
  const removed = slicing.units.find(u => u.removed && u.func_sym == OLD_FROB);
  const added = slicing.units.find(u => !u.removed && u.func_sym == NEW_FROB);
  ok(removed?.path == "widget.cpp", "The slicing has the lines the push removes, as units");
  ok(added && clusterOf(removed) == clusterOf(added),
     "which are in the cluster of what replaces them");

  // "Let widgets shrink": Shrink's declaration and definition, and unpoke.cpp,
  // whose includes and namespace aren't dataflow.
  await TestUtils.loadQuery("trydelta", "slice-rev:3b6f1c8f943932e006c9b03d8978879e05dfc4fc slice-format:svg");
  ok(sliceLinks().includes("/trydelta/source/widget.cpp#7"),
     "A landed commit's slicing links to the indexed revision's lines");
  ok(doc().querySelector(".query-result svg").textContent.includes("without dataflow"),
     "and has its changes without dataflow apart");
});

// A slicing's default view is a subway map (see subway.js): its functions'
// cards, their stations (statement blocks) with the lines (values) stopping
// there, and the lines along the calls between them, with a legend toggling
// them and a link to the dataflow graph.
add_task(async function test_TryDeltaSubway() {
  const head = await importPush();
  await TestUtils.loadPath(
    `/trydelta/query/default?q=${encodeURIComponent(`slice-rev:${head}`)}&rev=${head}`);
  await waitForCondition(() => doc().documentElement.dataset.subwayReady, "The subway map is drawn");
  const win = frame.contentWindow;
  const model = win.SUBWAY;
  ok(model.cards.length > 1 && model.lines.length > 0, "The map has the push's functions and values");
  const cards = [...doc().querySelectorAll(".sw-card")];
  is(cards.length, model.cards.length, "as cards");
  ok(doc().querySelector(`.sw-card a.sw-lno[href^="/trydelta/rev/${head}/"]`),
     "whose lines link to the push's head's");
  ok(doc().querySelector(".sw-card .sw-dot"), "with dots where values stop");
  const legend = doc().querySelector("#sw-legend");
  ok(legend.querySelector(".sw-legend-line"), "The legend lists the values");
  const graphLink = legend.querySelector("a.sw-dot-link");
  ok(decodeURIComponent(graphLink.getAttribute("href")).includes("slice-format:svg"),
     "and links to the dataflow graph");

  // (Hiding a line hides its tracks and edges.)
  const line = model.lines.find(l => doc().querySelector(`.sw-track[data-line="${l.id}"]`));
  ok(line, "A line has tracks");
  const box = legend.querySelector(`input[data-line="${line.id}"]`);
  box.checked = false;
  box.dispatchEvent(new win.Event("change"));
  ok(!doc().querySelector(`.sw-track[data-line="${line.id}"]`), "Unchecking it hides it");
  ok(!doc().querySelector(`.interactive-graph-edge-line-${line.id}`), "and its edges");
  box.checked = true;
  box.dispatchEvent(new win.Event("change"));
  ok(doc().querySelector(`.sw-track[data-line="${line.id}"]`), "and checking it shows it again");

  // (A line's dots are at the rows of the events reading or writing it.)
  const rowStops = (lno, line) => model.cards.some(card => card.stations.some(station =>
    station.rows.some(row => row.line == lno && (row.stops ?? []).includes(line))));
  ok([...doc().querySelectorAll(".sw-card .sw-dot")].every(dot =>
    rowStops(Number(dot.closest(".sw-row").querySelector(".sw-lno").textContent), Number(dot.dataset.line))),
     "Its dots are at the rows of their lines' events");

  // (Each of a family's lines has a color of its own.)
  for (const family of model.families) {
    const colors = model.lines.filter(l => l.family == family.id).map(l => win.SubwayMap.colors.get(l.id));
    is(new Set(colors).size, colors.length, `${family.label}'s lines have colors of their own`);
  }

  // The cards' code is highlighted as the push's source listings are, with
  // their symbols, whose context menus work.
  const token = [...doc().querySelectorAll(".sw-code [data-symbols]")].find(t => t.textContent == "mSize");
  ok(token, "The cards' code has its symbols");
  ok(doc().querySelector('.sw-code [class^="syn_"]'), "and is highlighted");
  ok(win.SYM_INFO[token.dataset.symbols], "and the symbols' SYM_INFO");
  TestUtils.click(token);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "A symbol's click shows its context menu");
  ok(menu.textContent.includes("trydelta::Widget::mSize"), "for it");
  doc().dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));

  // Hovering a station highlights its rows, and its lines' symbols' tokens.
  const station = [...doc().querySelectorAll(".sw-station")]
    .find(s => s.contains(token) && s.dataset.stops);
  TestUtils.dispatchMouseEvent("mouseover", station.querySelector(".sw-code"));
  ok(station.classList.contains("sw-station-hot"), "Hovering a station highlights it");
  ok(token.classList.contains("sw-brushed"), "and its lines' symbols' tokens");
  const brushed = [...doc().querySelectorAll(".sw-brushed")];
  ok(brushed.some(t => !station.contains(t)), "in the other cards too");
});

// A try push's page's excerpts of its changed definitions (see
// test_Explore.js's test_ExploreExcerpts) prefer the lines on its changes'
// dataflow, from its slicing (`slice-format:lines`; see
// tools/src/dataflow/impact.rs), by its rebased head's lines.
add_task(async function test_TryDeltaExcerpts() {
  const head = await importPush();
  const file = doc().querySelector('.explore-file[data-path="widget.cpp"]');
  ok(file?.dataset.excerpts, "The push's files have excerpts");
  const win = frame.contentWindow;
  const data = await (await win.fetch(file.dataset.excerpts, { cache: "no-store" })).json();
  ok(decodeURIComponent(data.dataflow || "").includes("slice-format:lines") &&
     data.dataflow.endsWith(`&rev=${head}`),
     "which name its slicing's lines on its changes' dataflow");
  const response = await win.fetch(data.dataflow, {
    headers: { Accept: "application/json" },
    cache: "no-store",
  });
  ok(response.ok, "which the pipeline-server gives");
  ok((await response.json())?.JsonValue?.value?.files, "by file");

  const row = [...file.querySelectorAll("tr[data-range]")]
    .find(r => r.querySelector(".explore-outline-name").textContent.includes("Frob"));
  TestUtils.dispatchMouseEvent("mouseover", row);
  await waitForCondition(() => doc().querySelector("#panel-excerpt .excerpt-added"),
                         "Hovering over a changed definition shows its excerpt");
  ok(doc().querySelector("#panel-excerpt .excerpt-removed"), "with the lines it removed");
});

add_task(async function test_TryDeltaUnanalyzedFiles() {
  const head = await importPush();

  await TestUtils.loadPath("/trydelta/query/default?q=greetPerson");
  ok(doc().querySelector('a[href$="/hello.js"]'),
     "The indexed revision has the symbols of a file the push changes");
  await TestUtils.loadPath(`/trydelta/query/default?q=greetPerson&rev=${head}`);
  ok(!doc().querySelector('a[href$="/hello.js"]'),
     "but in the push's context, when its analysis doesn't cover the file, the file has none");
  await TestUtils.loadPath(`/trydelta/rev/${head}/hello.js`);
  ok(doc().querySelector("#line-2"), "The file's page at the head shows it");
  ok(!doc().querySelector(".source-line span[data-symbols]"),
     "without the indexed revision's symbols, at its lines");
});

// The push's citations of web specs are its analysis's too, as the indexed
// revision's are, so that its pages' citations have the push's lines, and
// the algorithms they cite can be shown beside the code citing them at its
// head, compared with the code citing them that the spec tree's view has.
add_task(async function test_TryDeltaCitations() {
  const head = await importPush();

  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${UNPOKE}&rev=${head}`);
  ok(doc().querySelector(`a.query-line-number[href="/trydelta/rev/${head}/unpoke.cpp#8"]`),
     "The push's citations of a spec's steps are its analysis's");
  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${UNPOKE}`);
  ok(doc().querySelector('a[href$="/unpoke.cpp#7"]') && !doc().querySelector('a[href$="/unpoke.cpp#8"]'),
     "unlike the indexed revision's, at its lines");

  await TestUtils.loadPath(`/trydelta/rev/${head}/unpoke.cpp`);
  const citation = doc().querySelector(`#line-5 span[data-symbols="${UNPOKE}"]`);
  TestUtils.click(citation);
  let menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the head's citation");
  const beside = [...menu.querySelectorAll("a")]
    .find(a => a.textContent.includes("beside the code citing it"));
  const VIEW = `/trydelta/rev-algorithm/${head}/${UNPOKE}`;
  is(beside?.getAttribute("href"), VIEW,
     "which shows the algorithm beside the code citing it at the push's head");
  frame.contentWindow.ContextMenu.hide();

  await TestUtils.loadPath(VIEW);
  ok(doc().querySelector("#rendered-doc")?.textContent.includes("To unpoke"),
     "The view has the algorithm's steps");
  const [left, right] = [...doc().querySelectorAll("#sp-algorithm-0 .sp-citations")];
  const steps = doc().querySelector("#rendered-doc").getBoundingClientRect();
  ok(left.getBoundingClientRect().right <= steps.left &&
     steps.right <= right.getBoundingClientRect().left,
     "between the code citing it at the push's head and the code it's compared with");
  const step1 = [...left.querySelectorAll(".sp-citation")]
    .find(card => card.dataset.step == "1");
  ok(step1?.querySelector(`a[href="/trydelta/rev/${head}/unpoke.cpp#8"]`),
     "The push's citations link to its head's lines");
  ok(step1?.classList.contains("sp-citation-changed"), "and are marked as changed by it");
  ok(step1?.querySelector(".sp-citation-rows")?.textContent
       .includes("aWidget.Frob(-aWidget.Size(), false);"),
     "with the push's code");
  ok(right.querySelector('a[href^="/tests/source/spec_citations.cpp#"]'),
     "The code it's compared with is the spec tree's view's");

  // (The push's code's symbols' menus are of the push's head.)
  const frob = [...left.querySelectorAll(".sp-citation-rows span[data-symbols]")]
    .find(span => span.textContent == "Frob");
  TestUtils.click(frob);
  menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "The push's code's symbols have context menus");
  const links = [...menu.querySelectorAll("a")];
  ok(links.find(a => a.textContent.includes("Go to definition"))
       ?.getAttribute("href")?.startsWith(`/trydelta/rev/${head}/widget.cpp#`),
     "which jump to the push's definitions, at its head");
  const search = menu.querySelector(".icon-search")?.getAttribute("href");
  ok(search?.startsWith("/trydelta/query/default?q=symbol:") && search.endsWith(`&rev=${head}`),
     "and search in the push's context");
  frame.contentWindow.ContextMenu.hide();
});

// A try push's canvas (see tools/src/patch_canvas.rs and rev-canvas.js): the
// functions it adds or changes as cards of their code, connected by their
// calls, and those calling them or called by them, whose arrangement is the
// user's, kept in the browser and exported as JSON.
add_task(async function test_TryCanvas() {
  const head = await importPush();
  const canvas = doc().querySelector(".explore-semantic-head a.explore-canvas")?.getAttribute("href");
  is(canvas, `/trydelta/rev-canvas/${head}`, "The push's page links to its canvas");

  await TestUtils.loadPath(canvas);
  await waitForCondition(() => doc().documentElement.dataset.canvasReady, "The canvas is laid out");
  const card = name => [...doc().querySelectorAll(".rc-card")]
    .find(c => c.querySelector(".rc-name")?.textContent == name);
  const status = name => card(name)?.querySelector(".rc-badge")?.textContent;
  is(status("trydelta::Widget::Wobble"), "added", "A function it adds is a card");
  is(status("trydelta::WobbleTwice"), "added", "as are those of a file it adds");
  is(status("trydelta::Widget::Frob"), "changed", "and those it changes, even their signatures");
  is(status("trydelta::UseWidget"), "context", "and the functions calling them");
  ok(!card("trydelta::Widget::Shrink"), "but not the ones it doesn't change");
  const changed = card("trydelta::Unpoke").querySelectorAll(".rc-changed-line");
  is([...changed].map(row => row.querySelector(".rc-lineno").dataset.lineNumber).join(), "7,9",
     "A card's code marks the lines the push adds or changes");
  ok(card("trydelta::Unpoke").querySelector(".rc-lineno")?.getAttribute("href")
       .startsWith(`/trydelta/rev/${head}/unpoke.cpp#`), "which link to its head's lines");
  is(doc().querySelectorAll("#interactive-graph-container g.edge").length, 5,
     "The cards are connected by the calls among them");
  ok(doc().querySelector(".interactive-graph-buttons"), "and can be zoomed");
  is(doc().querySelectorAll(".rc-legend .rc-legend-item").length, 5,
     "A legend says what the cards' edges and the lines' backgrounds mean");

  // (The calls' lines are marked when hovering over them.)
  const edge = doc().querySelector(
    '#interactive-graph-container g.edge[data-symbols="_ZN8trydelta11WobbleTwiceERNS_6WidgetE->_ZN8trydelta6Widget6WobbleEv"]');
  edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  is([...card("trydelta::WobbleTwice").querySelectorAll(".rc-call-line")]
       .map(row => row.querySelector(".rc-lineno").dataset.lineNumber).join(), "6,7",
     "Hovering over a call marks the lines making it");
  edge.dispatchEvent(new MouseEvent("mouseout", { bubbles: true }));

  // (Its arrangement is the user's.)
  doc().querySelector(".rc-context-toggle").click();
  ok(!card("trydelta::UseWidget"), "The context can be hidden");
  card("trydelta::Widget::Wobble").querySelector(".rc-hide").click();
  ok(!card("trydelta::Widget::Wobble"), "and so can a card");
  card("trydelta::Unpoke").querySelector(".rc-collapse").click();
  ok(!card("trydelta::Unpoke").querySelector(".rc-rows"), "which can also show only its name");
  const exported = frame.contentWindow.RevCanvas.exported();
  is(exported.head, head, "The arrangement exports as JSON, of the push");
  ok(exported.model.endsWith(`/trydelta/rev-canvas/${head}/canvas.json`), "with its model's URL");
  is(JSON.stringify(exported.state.hidden), JSON.stringify(["_ZN8trydelta6Widget6WobbleEv"]),
     "and its hidden cards");
  is(exported.state.context, false, "and its other choices");

  await waitForCondition(() => doc().querySelector(".rc-status")?.textContent,
                         "It's kept in this browser");
  await TestUtils.loadPath(canvas);
  await waitForCondition(() => doc().documentElement.dataset.canvasReady, "The canvas is laid out again");
  ok(!card("trydelta::Widget::Wobble") && !card("trydelta::UseWidget"), "as it was arranged");
  doc().querySelector(".rc-toolbar button[title='Forget this arrangement']").click();
  ok(card("trydelta::Widget::Wobble") && card("trydelta::UseWidget"), "until it's reset");

  const response = await fetch(`/trydelta/rev-canvas/${head}/canvas.json`);
  is(response.headers.get("Access-Control-Allow-Origin"), "*",
     "Its model is JSON which other sites can read");
  is((await response.json()).nodes.length, 7, "with its cards");
});

// The canvas's dataflow clusters (see rev-canvas.js's `loadClusters`): its
// cards are badged with the clusters of the push's slicing, which a panel
// lists by commit and class, and which can be shown alone, or merged by class.
add_task(async function test_TryCanvasClusters() {
  const head = await importPush();
  await TestUtils.loadPath(`/trydelta/rev-canvas/${head}`);
  await waitForCondition(() => doc().documentElement.dataset.canvasClustersReady,
                         "The push's clusters are loaded");
  const card = name => [...doc().querySelectorAll(".rc-card")]
    .find(c => c.querySelector(".rc-name")?.textContent == name);
  const badges = name => [...(card(name)?.querySelectorAll(".rc-cluster-badge") ?? [])]
    .map(b => b.textContent).join();
  ok(badges("trydelta::Widget::Wobble"), "Cards are badged with their clusters");
  is(badges("trydelta::WobbleTwice"), badges("trydelta::Widget::Wobble"),
     "and Wobble's new caller is in its cluster");
  isnot(badges("trydelta::Unpoke"), badges("trydelta::Widget::Wobble"), "unlike Unpoke");
  ok(doc().querySelector(".rc-cluster-commit")?.textContent.includes(head.slice(0, 12)),
     "The panel lists them by commit");

  const show = cluster => doc().querySelector(`.rc-cluster-show[data-cluster="${cluster}"]`);
  const cluster = badges("trydelta::Widget::Wobble").split(",")[0];
  show(cluster).click();
  ok(card("trydelta::Widget::Wobble") && card("trydelta::WobbleTwice") && !card("trydelta::Unpoke"),
     "A cluster can be shown alone");
  show(cluster).click();
  ok(card("trydelta::Unpoke"), "and then all of them again");

  doc().querySelector(".rc-cluster-merge").click();
  ok(doc().querySelector('.rc-cluster-show[data-cluster^="g"]'),
     "Clusters can be merged by class");
  doc().querySelector(".rc-toolbar button[title='Forget this arrangement']").click();
});

// When the slicing fails, the canvas says why (the server's error, or the
// browser's), with a link to the slicing query's page.
add_task(async function test_TryCanvasClustersError() {
  const head = await importPush();
  await TestUtils.loadPath(`/trydelta/rev-canvas/${head}`);
  await waitForCondition(() => doc().documentElement.dataset.canvasClustersReady,
                         "The push's clusters are loaded");
  const win = frame.contentWindow;
  const note = () => doc().querySelector(".rc-cluster-note");

  // (As the pipeline-server reports a pipeline's error.)
  win.fetch = async () => new win.Response(
    'Error: StickyProblem(\n    ErrorDetails {\n        layer: DataLayer,\n' +
    '        message: "no \\"rev\\" here",\n    },\n)',
    { status: 500, statusText: "Internal Server Error" });
  await win.RevCanvas.loadClusters();
  ok(note().textContent.includes('500 Internal Server Error: no "rev" here'),
     `The server's error is shown: ${note().textContent}`);
  const link = note().querySelector("a.rc-cluster-query");
  ok(link?.getAttribute("href").startsWith("/trydelta/query/default?q=slice-rev%3A"),
     "with a link to the slicing query's page");
  ok(!link.getAttribute("href").includes("slice-format"), "rendered, not JSON");

  win.fetch = async () => { throw new win.TypeError("NetworkError when attempting to fetch resource."); };
  await win.RevCanvas.loadClusters();
  ok(note().textContent.includes("the request failed: NetworkError"),
     `and the browser's: ${note().textContent}`);
});

// A push whose stack has a commit which has landed since it diverged: its
// semantic analysis leaves that commit out (rather than its changes
// conflicting with the landed commit's), which its page says: 1a2b3c4d5e6f is
// based on the tree's first revision, its first commit is the second's
// changes, and its second commit uses them.
add_task(async function test_TryDeltaLanded() {
  await TestUtils.loadPath("/trydelta/try/1a2b3c4d5e6f");
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  const landed = doc().querySelector(".explore-landed");
  ok(landed?.textContent.includes("One of its commits has landed"), "Its page says a commit landed");
  ok(landed?.textContent.includes("Bug 2345 - Let widgets shrink."), "which one");
  const as = landed?.querySelector(".explore-landed-as");
  ok(as?.getAttribute("href").match(/^\/trydelta\/commit\/[0-9a-f]{40}$/),
     `and as what: ${as?.getAttribute("href")}`);
  const asPage = await (await fetch(as.getAttribute("href"), { cache: "no-store" })).text();
  ok(asPage.includes("Let widgets shrink"), "the tree's second revision");
  ok(doc().querySelector(".explore-semantic-head"),
     "The rest of its stack has semantic analysis");
  ok(!doc().querySelector(".explore-semantic-failure"), "which didn't fail");
});

// A push whose changes conflict with the indexed revision's can't be analyzed
// semantically, which its page says, with why: fedcba987654 is based on the
// tree's first revision, and declares Widget::Grow where the second declares
// Widget::Shrink.
add_task(async function test_TryDeltaFailure() {
  await TestUtils.loadPath("/trydelta/try/fedcba987654");
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  ok(!doc().querySelector(".explore-semantic-head"), "The push has no semantic analysis");
  const failure = doc().querySelector(".explore-semantic-failure")?.textContent ?? "";
  ok(failure.includes("failed in its rebase step"), "Its page says which step of it failed");
  ok(failure.includes("conflict with the indexed revision's"), "and why");
  ok(failure.includes("The files whose changes conflict: widget.h."), "with the files");
});
