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
