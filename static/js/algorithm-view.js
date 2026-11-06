/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// An algorithm's view beside the code citing its steps (see the tools'
// `format::format_algorithm`), with its sections' steps (`ALGORITHM_SECTIONS`,
// a JSON data block).  See the comment before its data in the tools for what
// this places, links and shows.

// (Choosing what to compare with goes there.)
document.querySelector(".sp-versus select")?.addEventListener("change", event => {
  event.target.form.submit();
});

(() => {
  const sections = ALGORITHM_SECTIONS;
  // A step's block: its text's, by its tokens' lines (`data-l`, but blanks',
  // ex: a paragraph's last, of the next line; not in the copies of called
  // algorithms' steps), as blocks' line ranges overlap (ex: a list item's,
  // without its end tag, goes to the next one's line), or else the block
  // whose range starts last before it.
  const blockOfLine = (doc, line) => {
    const token = [...doc.querySelectorAll(`[data-l="${line}"]`)]
      .find(token => token.textContent.trim() && !token.closest(".sp-inline-called"));
    const block = token && (token.closest("tr") || token.closest(".rd-block"));
    if (block) {
      return block;
    }
    let found = null;
    let start = -1;
    for (const mark of doc.querySelectorAll("[data-rd-lines]")) {
      const [first, last] = mark.dataset.rdLines.split("-").map(Number);
      if (first <= line && line <= last && first > start) {
        found = mark.closest("tr") || mark.parentElement;
        start = first;
      }
    }
    return found;
  };
  sections.forEach(({ steps }, n) => {
    const doc = document.getElementById(n ? `rendered-doc-${n}` : "rendered-doc");
    const asides = [...document.querySelectorAll(`#sp-algorithm-${n} > .sp-algorithm-view > .sp-citations`)];
    if (!doc || !asides.length) {
      return;
    }
    // Each column's cards, and their steps' blocks, which stick below the
    // page's header while their code shows.
    const columns = asides.map(aside => ({
      aside,
      cards: [...aside.querySelectorAll(":scope > .sp-citation")].map(element => {
        const line = steps[element.dataset.step];
        return { element, block: (line && blockOfLine(doc, line)) || null };
      }),
    }));
    const cards = columns.flatMap(({ cards }) => cards);
    for (const { block } of cards) {
      block?.classList.add("sp-step-sticky");
    }
    const blocks = [...new Set(cards.map(({ block }) => block).filter(Boolean))]
      .sort((a, b) => a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    // (The space before a step's block: after the step before it which has
    // code, at its list item's end (so its text sticks while its code shows),
    // or before its own sub-steps if it's one of them, or else before it.)
    const spaceBefore = (block, previous) => {
      const item = previous?.parentElement?.tagName == "LI" ? previous.parentElement : null;
      const [at, where] = !item ? [block, "before"]
        : item.contains(block) ? [previous, "after"] : [item, "append"];
      const row = (where == "append" ? at.lastElementChild : at)?.tagName == "TR";
      const space = document.createElement(row ? "tr" : "div");
      space.className = "sp-step-space";
      space.setAttribute("aria-hidden", "true");
      if (row) {
        space.append(document.createElement("td"));
      }
      at[where](space);
      return space;
    };
    const place = () => {
      // (Where the steps are, not where they're stuck.)
      doc.classList.add("sp-placing");
      placeSideCards({
        doc,
        columns,
        blocks,
        spaceClass: "sp-step-space",
        spaceBefore,
        // (A card continuing the one before it adjoins it.)
        gap: card => (card.element.classList.contains("sp-citation-continued") ? 0 : 6),
      });
      doc.classList.remove("sp-placing");
    };
    // Again when the view's size changes (but by its spaces), or a card's
    // (ex: its folded lines showing).
    let placed = null;
    const size = () => `${doc.clientWidth}x${doc.offsetHeight}`;
    const replace = () => {
      place();
      placed = size();
    };
    replace();
    const observer = new ResizeObserver(entries => {
      if (entries.some(entry => entry.target !== doc) || size() !== placed) {
        replace();
      }
    });
    observer.observe(doc);
    for (const { element } of cards) {
      observer.observe(element);
    }
    document.fonts?.ready.then(replace);
  });
  sections.forEach(({ anchor }, n) => {
    if (!anchor) {
      return;
    }
    for (const link of document.querySelectorAll(".sp-algorithm a[href*='#']")) {
      const hash = link.getAttribute("href").split("#").pop();
      if (decodeURIComponent(hash) !== anchor || link.closest(`#sp-algorithm-${n}`)) {
        continue;
      }
      const down = document.createElement("a");
      down.className = "sp-called-link";
      down.href = `#sp-algorithm-${n}`;
      down.title = "Go to this algorithm's section below";
      down.textContent = "↓";
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "sp-inline-toggle";
      toggle.dataset.section = n;
      toggle.title = "Show this algorithm's steps here";
      toggle.setAttribute("aria-expanded", "false");
      toggle.textContent = "▸";
      link.after(down, toggle);
    }
  });
  // A card's folded lines, and the lines after it, show on its buttons'
  // clicks (the latter's with the former).
  document.addEventListener("click", event => {
    const unfold = event.target.closest(".sp-citation-unfold");
    if (!unfold) {
      return;
    }
    const card = unfold.closest(".sp-citation");
    const groups = unfold.dataset.unfold == "after" ? ["fold", "after"] : ["fold"];
    for (const group of groups) {
      card.querySelector(`.sp-citation-${group}`)?.removeAttribute("hidden");
      card.querySelector(`.sp-citation-unfold[data-unfold="${group}"]`)?.remove();
    }
  });
  const shown = new WeakMap();
  document.addEventListener("click", event => {
    const toggle = event.target.closest(".sp-inline-toggle");
    if (!toggle) {
      return;
    }
    if (shown.has(toggle)) {
      shown.get(toggle).remove();
      shown.delete(toggle);
      toggle.textContent = "▸";
      toggle.setAttribute("aria-expanded", "false");
      return;
    }
    const n = Number(toggle.dataset.section);
    const called = document.createElement("div");
    called.className = "sp-inline-called";
    const head = document.createElement("div");
    head.className = "sp-inline-head";
    const go = document.createElement("a");
    go.href = `#sp-algorithm-${n}`;
    go.textContent = "↓ beside the code citing it";
    head.append(`${sections[n].name} `, go);
    called.append(head);
    for (const child of document.getElementById(`rendered-doc-${n}`).children) {
      called.append(child.cloneNode(true));
    }
    // (Without its section's spaces, nor sticking steps.)
    for (const elt of called.querySelectorAll(".blame-strip, [data-rd-lines], .sp-inline-called, .sp-step-space")) {
      elt.remove();
    }
    for (const elt of called.querySelectorAll(".sp-step-sticky")) {
      elt.classList.remove("sp-step-sticky");
    }
    for (const elt of called.querySelectorAll("[id]")) {
      elt.removeAttribute("id");
    }
    for (const copy of called.querySelectorAll(".sp-inline-toggle")) {
      copy.textContent = "▸";
      copy.setAttribute("aria-expanded", "false");
    }
    // (After the calling block, and the calls' already shown after it.)
    let at = toggle.closest(".rd-block") || toggle.parentElement;
    while (at.nextElementSibling?.classList.contains("sp-inline-called")) {
      at = at.nextElementSibling;
    }
    at.after(called);
    shown.set(toggle, called);
    toggle.textContent = "▾";
    toggle.setAttribute("aria-expanded", "true");
  });
  const map = document.getElementById("sp-algorithm-map");
  if (map) {
    const nodes = [...map.querySelectorAll("g.node[id^='sp-map-']")];
    const sectionOf = node => document.getElementById(`sp-algorithm-${node.id.slice("sp-map-".length)}`);
    const keepHeight = () => {
      document.documentElement.style.setProperty("--sp-map-height", `${map.offsetHeight}px`);
    };
    keepHeight();
    new ResizeObserver(keepHeight).observe(map);
    const showCurrent = () => {
      const bottom = map.getBoundingClientRect().bottom + 1;
      let current = nodes[0];
      for (const node of nodes) {
        if (sectionOf(node)?.getBoundingClientRect().top <= bottom) {
          current = node;
        }
      }
      for (const node of nodes) {
        node.classList.toggle("sp-map-current", node === current);
      }
    };
    let pending = false;
    document.addEventListener("scroll", () => {
      if (!pending) {
        pending = true;
        requestAnimationFrame(() => {
          pending = false;
          showCurrent();
        });
      }
    }, { passive: true });
    showCurrent();
    // A node's click goes to its section (the map has no symbols' menus on
    // clicks: see `data-own-context-menu`), and its right-click is a menu of
    // its algorithm's pages.
    for (const node of nodes) {
      node.addEventListener("click", () => {
        keepHeight();
        sectionOf(node)?.scrollIntoView();
      });
      node.addEventListener("contextmenu", event => {
        const section = sectionOf(node);
        if (!section || typeof ContextMenu == "undefined") {
          return;
        }
        event.preventDefault();
        const n = Number(node.id.slice("sp-map-".length));
        const link = selector => section.querySelector(selector)?.getAttribute("href");
        const items = [
          ["Go to its section", `#${section.id}`],
          ["Show it beside the code citing it, and the algorithms it calls",
           n ? link(".sp-called-view") : null],
          ["Show it in its spec", n ? link(".sp-called-rendered") : null],
        ].filter(([, href]) => href).map(([html, href]) => new MenuItem({
          html,
          href,
          preaction: () => ContextMenu.hide(),
          icon: "export-alt",
          section: "algorithm-map",
        }));
        ContextMenu.showItems(items, event);
      });
    }
  }
})();
