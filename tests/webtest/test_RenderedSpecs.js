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

add_task(async function test_RespecAlgorithmView() {
  // The ReSpec spec's algorithms, and its IDL members' steps, have views
  // beside the code citing them too.
  await TestUtils.loadPath("/tests/rendered/specs/respec/index.html");
  is(doc().querySelector("dfn#dom-gizmo-poke + a.sp-algorithm-link")?.getAttribute("href"),
     "/tests/algorithm/SPEC_examplerespec_dom@2Dgizmo@2Dpoke",
     "The rendered spec's IDL members' steps link to their views");

  await TestUtils.loadPath("/tests/algorithm/SPEC_examplerespec_dom@2Dgizmo@2Dpoke");
  const steps = frame.contentDocument.querySelector("#rendered-doc");
  ok(steps?.textContent.includes("method steps are"), "The view has the member's steps");
  ok(!steps?.textContent.includes("Acknowledgements"), "and only them");
  is(steps?.querySelector('a[href$="#dfn-gizmo"]')?.getAttribute("href"),
     "/tests/rendered/specs/respec/index.html#dfn-gizmo",
     "whose links to the spec's anchors are to its rendered view");
  ok(frame.contentDocument.querySelector("#sp-algorithm-1 .sp-called-heading")
       ?.textContent.includes("poke"),
     "The algorithm the steps call is below them");
  is(frame.contentDocument.querySelectorAll("#sp-algorithm-map g.edge").length, 2,
     "and the map has their calls of each other");
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

add_task(async function test_AlgorithmExcerpts() {
  // The code citing the example spec's "assemble" algorithm's steps, in a
  // function (tests/tests/files/spec_excerpts.cpp), and in another file.
  await TestUtils.loadPath("/tests/algorithm/SPEC_example_assemble");
  const cards = [...frame.contentDocument.querySelectorAll("#sp-citations > .sp-citation")]
    .filter(card => card.querySelector('a[href*="/spec_excerpts.cpp#"]'));
  is(cards.map(card => card.dataset.step).join(), ",1,2,2.1,2.2",
     "The function's citations of the algorithm and its steps are cards");
  const [whole, step1, step2, step21] = cards;

  // Each step's block goes down to its first card, if it's below it (after
  // the cards before it), with space after the step before it, whose text
  // sticks while its code shows.
  await frame.contentDocument.fonts.ready;
  await new Promise(resolve => frame.contentWindow.requestAnimationFrame(resolve));
  for (const card of [step1, step2]) {
    const first = [...frame.contentDocument.querySelectorAll("#sp-citations > .sp-citation")]
      .find(other => other.dataset.step == card.dataset.step);
    const sticky = [...doc().querySelectorAll(".sp-step-sticky")];
    const block = sticky.find(block => Math.abs(block.getBoundingClientRect().top -
                                                first.getBoundingClientRect().top) < 2);
    ok(block, `Step ${card.dataset.step}'s text is beside its first card`);
  }
  ok(doc().querySelector(".sp-step-space"), "with space before it");
  is(getComputedStyle(doc().querySelector(".sp-step-sticky")).position, "sticky",
     "and its text sticks while its code shows");

  ok(whole.querySelector(".sp-citation-head")?.textContent.includes("spec_excerpts::AssembleAll"),
     "The function's first card says its file and function");
  // (spec_citations.py's cite steps 1, 2.1 and 2.2 too, and go first.)
  ok(step1.querySelector(".sp-citation-head"),
     "and so does a card after another file's");
  ok(!step2.querySelector(".sp-citation-head") && step2.classList.contains("sp-citation-continued"),
     "but not a card continuing the one before it");
  is(step2.querySelector(".sp-citation-gutter")?.textContent.trim(), "step 2",
     "which has its step, in its gutter");

  const between = [...step1.querySelectorAll(".sp-citation-between")]
    .map(row => row.textContent.trim());
  ok(between.includes("Frob(node);"),
     `The few lines between a card and the one continuing it show, lightened: ${JSON.stringify(between)}`);

  ok(step2.textContent.includes("for (int i = 0; i < aCount; i++) {"),
     "A step's code goes on past its comments and blank lines");
  const fold = step2.querySelector(".sp-citation-fold");
  ok(fold?.hidden && fold.textContent.includes("Frob(aWidgets[i] + 12);"),
     "and a long excerpt's end folds");
  step2.querySelector('.sp-citation-unfold[data-unfold="fold"]').click();
  ok(!fold.hidden, "which shows on asking");
  ok(!step2.querySelector('.sp-citation-unfold[data-unfold="fold"]'), "once");

  const after = step21.querySelector(".sp-citation-after");
  ok(after?.hidden && after.textContent.includes("Frob(4);"),
     "The lines after an excerpt, up to the next citation, are there");
  step21.querySelector('.sp-citation-unfold[data-unfold="after"]').click();
  ok(!after.hidden, "and show on asking (in case its step goes on)");


  // A card continuing the one before it many lines on says how many (the
  // examplebs spec's poke, which only this file cites).
  await TestUtils.loadPath("/tests/algorithm/SPEC_examplebs_poke");
  const [, poke1, poke2] = [...frame.contentDocument.querySelectorAll("#sp-citations > .sp-citation")];
  is(poke2?.querySelector(".sp-citation-skip")?.textContent, "⋮ 5 lines",
     "A card continuing one many lines on says how many");
  ok(poke2 && !poke2.querySelector(".sp-citation-head"), "rather than its file and function");
  ok(poke1?.querySelector(".sp-citation-after")?.textContent.includes("Frob(4);"),
     "and the one before it has them");
});

add_task(async function test_AlgorithmView() {
  // The example spec's "poke" algorithm beside the code citing it and its
  // steps (tests/tests/files/spec_citations.cpp).
  const PATH = "/tests/algorithm/SPEC_example_concept@2Dwidget@2Dpoke";
  await TestUtils.loadPath(PATH);

  ok(doc().classList.contains("sp-algorithm"), "The page is an algorithm's view");
  ok(doc().textContent.includes("To poke"), "The algorithm is on the right");
  ok(!doc().querySelector("#infrastructure"), "and only it");
  const cards = [...frame.contentDocument.querySelectorAll(".sp-citation")];
  is(cards.map(card => card.dataset.step).join(), ",1,2",
     "The code citing the algorithm, and its steps, is on the left, in order");
  const aside = frame.contentDocument.querySelector("#sp-citations");
  ok(aside.getBoundingClientRect().right <= doc().getBoundingClientRect().left,
     "beside the algorithm");
  const step1 = cards[1];
  ok(step1.querySelector('a[href="/tests/source/spec_citations.cpp#13"]'),
     "A citation links to its line");
  ok(step1.textContent.includes("aWidget.state = 1;"), "and has its code, up to the next step's");
  ok(!step1.textContent.includes("Step 2"), "but not the next step's");
  ok(!cards[2].textContent.includes("Unpoke"), "nor past its block's end");
  // (Its source listing's rows, with their syntax highlighting and symbols.)
  const rows = step1.querySelector('.sp-citation-rows[data-tree="tests"]');
  ok(rows?.querySelector(".syn_comment"), "The code is its source listing's rows");
  is([...rows.querySelectorAll("a.sp-citation-lineno")].pop()?.getAttribute("href"),
     "/tests/source/spec_citations.cpp#14", "whose line numbers link to their lines");
  ok(!rows?.querySelector(".line-number, [id]"),
     "and aren't this page's lines");
  const state = rows?.querySelector('span[data-symbols^="F_"]');
  TestUtils.click(state);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "whose symbols have context menus");
  const gotoDef = [...menu.querySelectorAll("a")]
    .find(a => a.textContent.includes("Go to definition of spec_citations::Widget::state"));
  is(gotoDef?.getAttribute("href"), "/tests/source/spec_citations.cpp#8",
     "like the source listing's");
  frame.contentWindow.ContextMenu.hide();
  // (Each citation is next to its step's block, or after the citation before
  // it, if that's further down.)
  const block = [...doc().querySelectorAll("li .rd-block")]
    .find(block => block.textContent.includes("Set widget"));
  // (Its step's block goes down to it, if it's after the one before it,
  // which it adjoins, as it continues it.)
  ok(Math.abs(step1.getBoundingClientRect().top - block.getBoundingClientRect().top) < 2,
     "A step's citations are next to it");
  ok(step1.getBoundingClientRect().top >= cards[0].getBoundingClientRect().bottom - 1,
     "after the ones before it");

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
  const menu = doc.querySelector("#context-menu");
  ok(!menu || menu.style.display == "none",
     "Clicking a node doesn't show the symbols' context menu");

  // A node's right-click is a menu of its algorithm's pages.
  const right = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 });
  ok(!node(1)?.dispatchEvent(right), "A node's right-click is the page's");
  await waitForShown(doc.querySelector("#context-menu"), "with a menu");
  const hrefs = [...doc.querySelectorAll("#context-menu a")].map(a => a.getAttribute("href"));
  ok(hrefs.includes("#sp-algorithm-1"), `of its section: ${JSON.stringify(hrefs)}`);
  ok(hrefs.some(href => href?.startsWith("/tests/algorithm/")), "its own view");
  ok(!doc.querySelector("#context-menu").textContent.includes("Search for the substring"),
     "and not the symbols' menu's");
  frame.contentWindow.ContextMenu.hide();
});

// An algorithm's view's sections have their files' blame: the spechist tree's
// History 2 jiggle algorithm calls History 3's wiggle algorithm, of another
// file (see tests/spechist/revs/6), whose section's blocks have that file's
// blame.
add_task(async function test_AlgorithmViewSectionsBlame() {
  await TestUtils.loadPath("/spechist/algorithm/SPEC_hist2_concept@2Dwidget@2Djiggle");
  const doc = frame.contentDocument;
  is(frame.contentWindow.RENDERED_BLAME_INFOS?.length, 2,
     "The view has the blame of both of its sections' files");
  const called = doc.querySelector("#sp-algorithm-1");
  ok(called?.textContent.includes("Tilt"), "The algorithm it calls is below it");
  ok(doc.querySelector('#sp-algorithm-0 .blame-strip[data-hb-ctx="0"]'),
     "Its blocks have strips of its file's blame");
  const strip = called?.querySelector('.blame-strip[data-hb-ctx="1"]');
  ok(strip, "and the algorithm it calls has its own file's");

  const popup = doc.querySelector("#blame-popup");
  frame.contentWindow.BlamePopup.triggerElement = null;
  TestUtils.dispatchMouseEvent("mouseenter", strip, { bubbles: true });
  await waitForCondition(
    () => popup.style.display != "none" &&
      frame.contentWindow.BlamePopup.popupOwner == strip &&
      popup.querySelector(".rd-replica"),
    "Its blocks' popups are shown");
  await waitForCondition(
    () => popup.textContent.includes("Add the jiggling and wiggling specs"),
    "with the commits of its file's history");
});

add_task(async function test_RenderedSpecContextMenu() {
  // A rendered spec's definitions and links to definitions have the context
  // menu (see the tools' `format::annotate_spec_symbols`), whose "Go to
  // definition" goes to the definition in the rendered view, and a link's,
  // which its click shows rather than following it, can follow it.
  await TestUtils.loadPath(`${BASE}/gadgets.html`);
  const menu = frame.contentDocument.querySelector("#context-menu");
  const items = () => [...menu.querySelectorAll("a")].map(a => a.textContent.trim());

  TestUtils.click(frame.contentDocument.querySelector('dfn[id="concept-gadget"]'));
  await waitForShown(menu, "A definition's click shows the context menu");
  ok(items().some(text => text.startsWith("Search for")), `with a search: ${items().join(", ")}`);
  ok(!items().some(text => text.startsWith("Go to definition")), "but no going to itself");
  // (But anchoring the page's URL to it, without going to it.)
  frame.contentWindow.notReloaded = true;
  const anchor = [...menu.querySelectorAll("a")].find(a => a.textContent.trim().startsWith("Anchor to"));
  is(anchor?.textContent.trim(), "Anchor to gadget", "and anchoring the page's URL to it");
  anchor.click();
  is(frame.contentWindow.location.hash, "#concept-gadget", "which anchoring does");
  ok(frame.contentWindow.notReloaded, "without reloading the page");
  TestUtils.click(frame.contentDocument.body);

  const link = frame.contentDocument.querySelector('a[data-symbols="SPEC_example_concept@2Dwidget"]');
  TestUtils.click(link);
  await waitForShown(menu, "A link to a definition's click shows the context menu");
  is(frame.contentWindow.location.pathname, `${BASE}/gadgets.html`, "rather than following it");
  const follow = [...menu.querySelectorAll("a")].find(a => a.textContent.includes("Follow the link"));
  is(follow?.getAttribute("href"), link.getAttribute("href"), "which can follow it");
  const def = [...menu.querySelectorAll("a")].find(a => a.textContent.includes("Go to definition"));
  is(def?.getAttribute("href"), `${BASE}#concept-widget`,
     "and go to the definition, in the rendered view rather than its source");
});
