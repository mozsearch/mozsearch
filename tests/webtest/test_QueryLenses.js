"use strict";

// /query/'s lenses (see navigation_panel.liquid), at the top of the panel:
// "changes" shows the results' history digests (see test_QueryRecency.js),
// which are hidden at first, and is remembered (and applied before the
// results load; see `EarlyLayout` in settings.js).
add_task(async function test_QueryChangesLens() {
  await TestUtils.loadQuery("searchfox", "blot_cell");
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("lens-changes"));
  await TestUtils.loadQuery("searchfox", "blot_cell");

  let doc = frame.contentDocument;
  let lens = doc.querySelector('.panel .lens-toggle[data-lens="changes"]');
  ok(lens, "There's a changes lens");
  is(lens.getAttribute("aria-pressed"), "false", "which is off at first");
  const blot = doc.querySelector(".query-result .query-recency[title]");
  ok(blot, "The results have history digests");
  is(frame.contentWindow.getComputedStyle(blot).display, "none", "which are hidden");

  TestUtils.click(lens);
  is(lens.getAttribute("aria-pressed"), "true", "Clicking the lens turns it on");
  isnot(frame.contentWindow.getComputedStyle(blot).display, "none", "and shows them");

  await TestUtils.loadQuery("searchfox", "blot_cell");
  doc = frame.contentDocument;
  lens = doc.querySelector('.panel .lens-toggle[data-lens="changes"]');
  is(lens.getAttribute("aria-pressed"), "true", "Later pages remember it");
  ok(doc.documentElement.classList.contains("lens-changes"), "and show the digests");

  TestUtils.click(lens);
  is(frame.contentWindow.localStorage.getItem(frame.contentWindow.storageKey("lens-changes")), "0",
     "Clicking it again turns it off for later pages too");
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("lens-changes"));
});

// "matches" chooses how the results' hits look (see mozsearch.css's
// `html.lens-matches-*`): "muted" at first, and the choice is remembered.
add_task(async function test_QueryMatchesLens() {
  await TestUtils.loadQuery("searchfox", "blot_cell");
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("lens-matches"));
  await TestUtils.loadQuery("searchfox", "blot_cell");

  let doc = frame.contentDocument;
  const win = () => frame.contentWindow;
  const choices = () => [...doc.querySelectorAll('.panel .lens-choice[data-lens="matches"]')];
  const choose = choice => TestUtils.click(choices().find(b => b.dataset.choice == choice));
  is(JSON.stringify(choices().map(b => b.dataset.choice)),
     JSON.stringify(["muted", "only-bold", "grayscale", "blue", "bold"]), "There's a matches lens of choices");
  is(JSON.stringify(choices().map(b => b.getAttribute("aria-pressed"))),
     JSON.stringify(["true", "false", "false", "false", "false"]), "whose first is chosen at first");
  ok(doc.documentElement.classList.contains("lens-matches-muted"), "as the page's class says");
  const hit = doc.querySelector(".query-result mark.query-hit");
  ok(hit, "The results have hits");
  const background = win().getComputedStyle(hit).backgroundColor;
  // (A colored token without a hit, ex: a keyword, and a definition's.)
  const token = [...doc.querySelectorAll(".query-result :is(.syn_reserved, .syn_type)")]
    .find(span => !span.querySelector("mark"));
  ok(token, "and highlighted tokens");
  // (As RGB bytes, through a canvas: the computed colors are `color(srgb ...)`
  // when muted and `oklch(...)` when grayscale.)
  const colors = () => {
    const context = doc.createElement("canvas").getContext("2d");
    context.fillStyle = win().getComputedStyle(token).color;
    context.fillRect(0, 0, 1, 1);
    return [...context.getImageData(0, 0, 1, 1).data.slice(0, 3)];
  };
  const gray = ([r, g, b]) => Math.max(r, g, b) - Math.min(r, g, b) < 3;
  ok(!gray(colors()), `Muted, its color is still a color: ${colors()}`);

  choose("grayscale");
  ok(gray(colors()), `Grayscale, it's gray: ${colors()}`);

  const def = doc.querySelector(".query-result .syn_def");
  choose("only-bold");
  if (def) {
    is(win().getComputedStyle(def).fontWeight, "400", "Only-bold, a definition isn't bold");
  }
  is(win().getComputedStyle(hit).fontWeight, "700", "but the hits are");

  choose("bold");
  ok(doc.documentElement.classList.contains("lens-matches-bold") &&
     !doc.documentElement.classList.contains("lens-matches-muted"), "Clicking another chooses it");
  isnot(win().getComputedStyle(hit).backgroundColor, background, "and changes the hits");

  await TestUtils.loadQuery("searchfox", "blot_cell");
  doc = frame.contentDocument;
  is(choices().find(b => b.getAttribute("aria-pressed") == "true")?.dataset.choice, "bold",
     "Later pages remember it");
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("lens-matches"));
});

// The results' files' paths, as /search/'s, have no spaces between their
// segments and separators (whose margins space them).
add_task(async function test_QueryPathSpacing() {
  await TestUtils.loadQuery("searchfox", "blot_cell");
  const path = frame.contentDocument.querySelector(".query-result h3.path");
  ok(path, "The results have paths");
  ok(path.textContent.trim().includes("/") && !/\s/.test(path.textContent.trim()),
     `without spaces: ${JSON.stringify(path.textContent)}`);
});

// Hierarchical facets' values (see facet_value.liquid): each with nested
// values has a button showing and hiding them, the facet's label has buttons
// showing all of its values or only the biggest at the top, and selected
// values always show.
add_task(async function test_FacetHierarchy() {
  await TestUtils.loadQuery("searchfox", "ustr");

  let doc = frame.contentDocument;
  let facet = doc.querySelector('.facet[data-facet="dir"]');
  ok(facet, "The results have a directory facet");
  const groups = () => [...facet.querySelectorAll(".facet-value-group")];
  const parent = groups().find(group => group.querySelector(":scope > .facet-nested"));
  ok(parent, "with nested values");
  const nested = parent.querySelector(":scope > .facet-nested");
  const expand = parent.querySelector(":scope > .facet-value-row > .facet-expand");
  const wasShown = !nested.hidden;
  is(expand.getAttribute("aria-expanded"), String(wasShown), "Its button says whether they show");
  TestUtils.click(expand);
  is(nested.hidden, wasShown, "and clicking it shows or hides them");

  TestUtils.click(facet.querySelector(".facet-expand-all"));
  ok(groups().every(group => !group.hidden) &&
     [...facet.querySelectorAll(".facet-nested")].every(n => !n.hidden),
     "The label's + shows all of the values");
  TestUtils.click(facet.querySelector(".facet-collapse-all"));
  ok([...facet.querySelectorAll(".facet-nested")].every(n => n.hidden),
     "and its − hides the nested ones");

  // A nested value selected (ex: by a link) shows.
  const value = nested.querySelector(".facet-value").dataset.value;
  await TestUtils.loadQuery("searchfox", "ustr", { "facet-dir": value });
  doc = frame.contentDocument;
  facet = doc.querySelector('.facet[data-facet="dir"]');
  const button = [...facet.querySelectorAll(".facet-value")].find(b => b.dataset.value == value);
  is(button.getAttribute("aria-pressed"), "true", "The URL's nested value is selected");
  ok(button.checkVisibility(), "and shows");
});
