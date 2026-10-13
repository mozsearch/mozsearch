"use strict";

// The rendered view of the test tree's Wattsi spec (see
// tests/tests/files/specs/source and tools/src/rendered_specs), which is in
// two pages, the second starting at its heading with a `split-filename`, like
// the multipage HTML Standard's.
const BASE = "/tests/rendered/specs/source";

function doc() {
  return frame.contentDocument.querySelector("#rendered-doc");
}

add_task(async function test_RenderedSpec() {
  await TestUtils.loadPath(BASE);

  ok(doc().classList.contains("rendered-spec"), "The page is a rendered spec");
  is(doc().querySelector("h2#infrastructure .sp-secno")?.textContent, "1",
     "Headings have their section numbers");
  ok(doc().querySelector("dfn#concept-widget"), "Definitions have their ids");
  const xref = doc().querySelector('a[href="#concept-widget"]');
  is(xref?.id, "widgets:concept-widget",
     "Cross-references link to their definitions, with Wattsi's ids");
  ok(doc().querySelector('span[data-l="9"][data-c="32"]'),
     "Text says where it is in the source");
  ok(!doc().textContent.includes("developer note"),
     "The developers' edition's content isn't in it");

  // The table of contents links to the sections on their pages.
  const toc = [...doc().querySelectorAll(".sp-toc a")].map(a => a.getAttribute("href"));
  ok(toc.includes("#widgets"), "The table of contents links to the page's sections");
  ok(toc.includes(`${BASE}/gadgets.html#gadgets`),
     "The table of contents links to the other page's sections");
  ok(!doc().querySelector("h3#gadgets"), "The other page's sections aren't on the page");

  const next = frame.contentDocument.querySelector(".sp-pages .sp-next");
  is(next?.getAttribute("href"), `${BASE}/gadgets.html`, "The page links to the next one");
  ok(next?.textContent.startsWith("Gadgets"), "The link has the next page's title");
});

add_task(async function test_RenderedSpecPage() {
  await TestUtils.loadPath(`${BASE}/gadgets.html`);

  ok(doc().querySelector("h3#gadgets"), "The page starts at its heading");
  ok(!doc().querySelector("h2#infrastructure"), "The first page's sections aren't on the page");
  is(doc().querySelector('[id="gadgets:concept-widget"]')?.getAttribute("href"),
     `${BASE}#concept-widget`,
     "Cross-references to the other page's definitions link to them there");
  is(doc().querySelector("dfn#node a")?.getAttribute("href"),
     "https://dom.spec.whatwg.org/#concept-node",
     "A definition deferring to another spec's term links to it");
  ok(doc().querySelector("pre.sp-idl"), "Classes are kept, prefixed");

  const prev = frame.contentDocument.querySelector(".sp-pages .sp-prev");
  is(prev?.getAttribute("href"), BASE, "The page links to the one before it");
  is(frame.contentDocument.querySelector("#panel-source-listing").getAttribute("href"),
     "/tests/source/specs/source#26",
     "The panel links to the page's first line in the source listing");
});

add_task(async function test_RenderedSpecLinks() {
  // Anchors and lines go to their pages.
  await TestUtils.loadPath(`${BASE}/anchor/assemble`);
  is(frame.contentWindow.location.pathname, `${BASE}/gadgets.html`,
     "An anchor's link goes to its page");
  is(frame.contentWindow.location.hash, "#assemble", "with the anchor");

  await TestUtils.loadPath(`${BASE}/line/34`);
  is(frame.contentWindow.location.pathname, `${BASE}/gadgets.html`,
     "A line's link goes to its page");
  is(frame.contentWindow.location.hash, "#34", "with the line");

  await TestUtils.loadPath(`${BASE}/line/13`);
  is(frame.contentWindow.location.pathname, BASE, "A line on the first page goes to it");

  // Links to the first page with another page's anchor go there.
  await TestUtils.loadPath(`${BASE}#concept-gadget`);
  await waitForCondition(
    () => frame.contentWindow.location.pathname == `${BASE}/gadgets.html` &&
      frame.contentDocument.readyState == "complete",
    "A link to another page's anchor goes to its page");
  is(frame.contentWindow.location.hash, "#concept-gadget", "with the anchor");

  // The source listing links to the rendered view, at the selected line.
  await TestUtils.loadPath("/tests/source/specs/source#34");
  const rendered = frame.contentDocument.querySelector("#panel-rendered-view");
  ok(rendered, "The source listing links to the rendered view");
  is(rendered.getAttribute("href"), `${BASE}/line/34`,
     "The link is to the selected line's page");
});

add_task(async function test_RenderedEcmarkup() {
  // The test tree's ecmarkup spec (tests/tests/files/specs/ecma/spec.html),
  // which imports a file.
  const SPEC = "/tests/rendered/specs/ecma/spec.html";
  await TestUtils.loadPath(SPEC);

  ok(doc().classList.contains("rendered-spec"), "The page is a rendered spec");
  is(doc().querySelector("#sec-tostring h3 .sp-secno")?.textContent, "1.1",
     "Clauses have their numbers");
  ok(doc().querySelector("li#step-tostring-else"), "Algorithms' steps have their ids");
  ok(doc().querySelector("#sec-tostring .sp-emu-alg var"), "ecmarkdown's variables are formatted");
  is(doc().querySelector('#sec-terms p a[href="#value"]')?.textContent, "Values",
     "Terms are autolinked");
  ok(doc().querySelector("#prod-Thing"), "Productions have their anchors");
  is([...doc().querySelectorAll("#sec-grammar p a")].map(a => a.textContent).join(),
     "1.1,ToString", "Empty cross-references have their targets' numbers or names");
  ok(doc().querySelector("#sec-imported"), "Imported files' content is in the spec");
  is(doc().querySelector(".sp-imported a")?.getAttribute("href"),
     "/tests/rendered/specs/ecma/imported.html", "and links to their own views");

  // An imported file's view is its part of the spec.
  await TestUtils.loadPath("/tests/rendered/specs/ecma/imported.html");
  is(doc().querySelector("#sec-imported h2 .sp-secno")?.textContent, "4",
     "An imported file's clauses have their numbers in the spec");
  is(doc().querySelector('a[href$="#sec-tostring"]')?.getAttribute("href"), `${SPEC}#sec-tostring`,
     "Its references to the spec's other definitions link to them there");
  ok(doc().querySelector('span[data-l="5"]'), "Its text says where it is in its file");

  await TestUtils.loadPath("/tests/source/specs/ecma/imported.html");
  is(frame.contentDocument.querySelector("#panel-rendered-view")?.getAttribute("href"),
     "/tests/rendered/specs/ecma/imported.html",
     "The imported file's source listing links to its rendered view");
});

add_task(async function test_RenderedBikeshed() {
  // The test tree's Bikeshed spec (tests/tests/files/specs/example.bs), whose
  // rendering is Bikeshed's (tests/tests/example-bs-output.html, which
  // spec-analyze puts in the index).
  const SPEC = "/tests/rendered/specs/example.bs";
  await TestUtils.loadPath(SPEC);

  ok(doc().classList.contains("rendered-spec"), "The page is a rendered spec");
  is(doc().querySelector("h2#widgets .sp-secno")?.textContent.trim(), "2.",
     "Headings have Bikeshed's numbers");
  ok(doc().querySelector("dfn#poke"), "Definitions have Bikeshed's ids");
  ok(!doc().querySelector("script, style"), "Bikeshed's scripts and styles aren't in it");
  const mark = [...doc().querySelectorAll("[data-rd-lines]")]
    .find(mark => mark.parentElement.textContent.includes("To poke"));
  is(mark?.dataset.rdLines, "15-15", "Blocks are of their lines in the source");
  ok(doc().querySelector('span[data-l="15"][data-c="0"]'),
     "Text that's verbatim in the source says where it is");
  ok(!frame.contentDocument.querySelector("#panel-permalink"),
     "Only the latest version has a rendered view");

  ok(doc().querySelector("dfn#gizmo"), "Included files' content is in the spec");
  is(doc().querySelector(".sp-imported a")?.getAttribute("href"),
     "/tests/rendered/specs/example-gizmos.md", "and links to their own views");

  await TestUtils.loadPath("/tests/source/specs/example.bs");
  is(frame.contentDocument.querySelector("#panel-rendered-view")?.getAttribute("href"), SPEC,
     "The source listing links to the rendered view");

  // An included file's view (even of Markdown) is its part of the spec.
  await TestUtils.loadPath("/tests/rendered/specs/example-gizmos.md");
  ok(doc().querySelector("dfn#gizmo"), "An included file's view has its content");
  ok(!doc().querySelector("h2#widgets"), "and only its");
  is(doc().querySelector('a[href$="#widget"]')?.getAttribute("href"), `${SPEC}#widget`,
     "Its references to the spec's other definitions link to them there");
  ok(doc().querySelector('span[data-l="3"]'), "Its text says where it is in its file");
});

add_task(async function test_RenderedRespec() {
  // The test tree's ReSpec spec (tests/tests/files/specs/respec/index.html),
  // rendered as ReSpec renders it, offline.
  const SPEC = "/tests/rendered/specs/respec/index.html";
  await TestUtils.loadPath(SPEC);

  ok(doc().classList.contains("rendered-spec"), "The page is a rendered spec");
  is(doc().querySelector("h2#x1-introduction .sp-secno")?.textContent, "1.",
     "Headings have ReSpec's numbers and ids");
  ok(doc().querySelector("#conformance h2#x4-conformance"),
     "and so do the headings ReSpec generates");
  ok(doc().querySelector("dfn#dfn-gizmo"), "Definitions have ReSpec's ids");
  ok(doc().querySelector("pre dfn#dom-gizmooptions"), "and so do IDL blocks'");
  is(doc().querySelector('code a[href="#dom-gizmo-poke"]')?.textContent, "poke()",
     "Shorthands are shown as ReSpec shows them, linked");
  ok(doc().querySelector("dt#bib-dom"), "Cited specs are in the references");
  is(doc().querySelector('a[href="/tests/rendered/specs/example.bs#gadget"]')?.textContent, "Gadget",
     "References to other specs' terms link to them (by the index's analysis)");
  ok(doc().querySelector('span[data-l="19"]'), "Text says where it is in the source");
  ok(frame.contentDocument.querySelector(".info-box-obsolete-spec"),
     "The obsolete spec's view says it's obsolete");

  await TestUtils.loadPath("/tests/source/specs/respec/index.html");
  is(frame.contentDocument.querySelector("#panel-rendered-view")?.getAttribute("href"), SPEC,
     "The source listing links to the rendered view");
});

add_task(async function test_ObsoleteSpec() {
  // The test tree's ReSpec spec is obsolete, superseded by its Bikeshed spec
  // (see tests/tests/specs.toml).
  await TestUtils.loadPath("/tests/source/specs/respec/index.html");
  const box = frame.contentDocument.querySelector(".info-box-obsolete-spec");
  ok(box?.textContent.includes("examplerespec is obsolete"),
     "An obsolete spec's source listing has an info box saying so");
  is(box?.querySelector("a")?.getAttribute("href"), "/tests/rendered/specs/example.bs",
     "which links to its successor's rendered view");

  await TestUtils.loadPath("/tests/source/specs/example.bs");
  ok(!frame.contentDocument.querySelector(".info-box-obsolete-spec"),
     "Other specs' source listings don't");
});

add_task(async function test_AlgorithmView() {
  // The example spec's "poke" algorithm beside the code citing it and its
  // steps (tests/tests/files/spec_citations.cpp).
  const PATH = "/tests/algorithm/SPEC_example_concept@2Dwidget@2Dpoke";
  await TestUtils.loadPath(PATH);

  ok(doc().classList.contains("sp-algorithm"), "The page is an algorithm's view");
  ok(doc().textContent.includes("To poke"), "The algorithm is on the left");
  ok(!doc().querySelector("#infrastructure"), "and only it");
  const cards = [...frame.contentDocument.querySelectorAll(".sp-citation")];
  is(cards.map(card => card.dataset.step).join(), ",1,2",
     "The code citing the algorithm, and its steps, is on the right, in order");
  const step1 = cards[1];
  ok(step1.querySelector('a[href="/tests/source/spec_citations.cpp#13"]'),
     "A citation links to its line");
  ok(step1.textContent.includes("aWidget.state = 1;"), "and has its code, up to the next step's");
  ok(!step1.textContent.includes("Step 2"), "but not the next step's");
  // (Each citation is next to its step's block, or after the citation before
  // it, if that's further down.)
  const block = [...doc().querySelectorAll("li .rd-block")]
    .find(block => block.textContent.includes("Set widget"));
  const expected = Math.max(block.getBoundingClientRect().top,
                            cards[0].getBoundingClientRect().bottom + 6);
  ok(Math.abs(step1.getBoundingClientRect().top - expected) < 2,
     "A step's citations are next to it, or after the ones before it");

  // The rendered spec links to the view from the algorithm's definition.
  await TestUtils.loadPath("/tests/rendered/specs/source");
  is(doc().querySelector("dfn#concept-widget-poke + a.sp-algorithm-link")?.getAttribute("href"),
     PATH, "The rendered spec's algorithms link to their views");
});

add_task(async function test_AlgorithmViewCalls() {
  // The Bikeshed spec's Gadget.poke() method steps call its "poke" algorithm,
  // which the view shows below them, beside the code citing it, linked from
  // the call.
  await TestUtils.loadPath("/tests/algorithm/SPEC_examplebs_dom@2Dgadget@2Dpoke");
  const called = frame.contentDocument.querySelector("#sp-algorithm-1");
  ok(called?.querySelector(".sp-called-heading")?.textContent.includes("poke"),
     "The algorithm the steps call is below them");
  ok(called?.querySelector(".sp-algorithm")?.textContent.includes("Set widget"),
     "with its steps");
  ok(called?.querySelector(".sp-citations"), "beside the code citing it");
  is(called?.querySelector(".sp-called-view")?.getAttribute("href"),
     "/tests/algorithm/SPEC_examplebs_poke", "and a link to its own view");
  const down = frame.contentDocument.querySelector("#sp-algorithm-0 a.sp-called-link");
  is(down?.getAttribute("href"), "#sp-algorithm-1", "The call links to it");
});

add_task(async function test_AlgorithmInlineCall() {
  // A call can show the algorithm it calls in place, after the calling block.
  await TestUtils.loadPath("/tests/algorithm/SPEC_examplebs_dom@2Dgadget@2Dpoke");
  const doc = frame.contentDocument;
  const toggle = doc.querySelector("#sp-algorithm-0 .sp-inline-toggle");
  ok(toggle, "The call can show its algorithm's steps in place");
  toggle?.click();
  const shown = doc.querySelector("#sp-algorithm-0 .sp-inline-called");
  ok(shown?.textContent.includes("Set widget"), "which it does");
  ok(shown?.previousElementSibling?.contains(toggle), "after the calling block");
  is(shown?.querySelector(".sp-inline-head a")?.getAttribute("href"), "#sp-algorithm-1",
     "with a link to its section");
  is(shown?.querySelectorAll("[id], [data-rd-lines], .blame-strip").length, 0,
     "without its section's ids or line markers");
  is(toggle?.getAttribute("aria-expanded"), "true", "The toggle says it's shown");
  toggle?.click();
  ok(!doc.querySelector("#sp-algorithm-0 .sp-inline-called"), "and hides it again");
});

add_task(async function test_AlgorithmMap() {
  // The view's map of its sections' algorithms and their calls, which stays at
  // the top, with the node of the section one has scrolled to highlighted, and
  // whose nodes scroll to their sections.
  await TestUtils.loadPath("/tests/algorithm/SPEC_examplebs_dom@2Dgadget@2Dpoke");
  const doc = frame.contentDocument;
  const map = doc.querySelector("#sp-algorithm-map");
  ok(map, "The view has a map");
  is(map?.querySelectorAll("g.node").length, 2, "of its sections' algorithms");
  is(map?.querySelector("#sp-map-1 text")?.textContent, "poke",
     "by their names, without their spec's shortname");
  is(map?.querySelectorAll("g.edge").length, 1, "and the call between them");
  ok(!map?.querySelector("[data-symbols]"), "whose nodes aren't symbols");
  const node = n => doc.querySelector(`#sp-map-${n}`);
  ok(node(0)?.classList.contains("sp-map-current"),
     "The first section's node is highlighted");

  // (Room to scroll the second section up to the map.)
  doc.body.style.paddingBottom = "200vh";
  node(1)?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await waitForCondition(() => node(1)?.classList.contains("sp-map-current"),
                         "Clicking a node highlights it");
  const section = doc.querySelector("#sp-algorithm-1").getBoundingClientRect();
  const bottom = map.getBoundingClientRect().bottom;
  ok(section.top >= bottom - 1 && section.top < bottom + 20,
     "and scrolls to its section, just below the map");
  ok(!node(0)?.classList.contains("sp-map-current"),
     "The first section's node isn't highlighted then");
});
