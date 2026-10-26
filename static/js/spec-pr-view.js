/**
 * A spec's rendered page with its open pull requests' changes beside it
 * (`/{tree}/rendered-prs/{path}`; see `format::format_rendered`'s `with_prs`
 * and rendered_prs.liquid in tools/).
 *
 * Each pull request's entry in the list before the page (`.sp-pr-entry`) has
 * its data (`data-pr`), with the URL of its changes of the page (`blocks`;
 * see `format::spec_pr_blocks_json`), which this fetches, saying in the entry
 * how many there are, or why there are none (ex: its changes conflict with
 * the trunk's, or a Bikeshed spec's need it imported).  Each change is a card
 * in the column beside the page (`.sp-pr-column`), with its pull request's
 * version of the blocks it changes, beside the trunk's block with its
 * `anchor` line (by the blocks' `data-rd-lines`): the page's blocks are
 * pushed down with spaces where the cards before them need the room, and the
 * cards beside the same block are stacked, as the algorithm view's citations
 * are (see `write_algorithm_view`).  A card has its pull request's header
 * when the card before it is another pull request's, or when the last one
 * with its header is more than a screen above.  Hovering over a card marks
 * the trunk's blocks it changes.  Filtering the entries with the facets (see
 * facets.js) hides their cards.
 */
(function () {
  const doc = document.getElementById("rendered-doc");
  const column = document.querySelector(".sp-pr-column");
  const entries = [...document.querySelectorAll(".sp-pr-entry")];
  if (!doc || !column || !entries.length) {
    return;
  }

  // The trunk's blocks' lines.
  const marks = [...doc.querySelectorAll("[data-rd-lines]")].map(mark => {
    const [first, last] = mark.dataset.rdLines.split("-").map(Number);
    return { first, last, block: mark.closest("tr") || mark.parentElement };
  }).filter(mark => mark.first);
  // The block with a line: the one whose lines have it and start last, or
  // else the last one before it, or the first.
  function blockOfLine(line) {
    let containing = null;
    let before = null;
    for (const mark of marks) {
      if (mark.first <= line && line <= mark.last &&
          (!containing || mark.first >= containing.first)) {
        containing = mark;
      }
      if (mark.first <= line && (!before || mark.first >= before.first)) {
        before = mark;
      }
    }
    return (containing || before || marks[0])?.block || null;
  }
  function blocksOfLines(first, last) {
    return marks.filter(mark => mark.first <= last && mark.last >= first).map(mark => mark.block);
  }

  const plural = (n, noun) => `${n} ${noun}${n == 1 ? "" : "s"}`;
  const cards = [];

  function card(pr, change) {
    const element = document.createElement("div");
    element.className = `sp-pr-card sp-pr-${change.kind}`;
    element.dataset.pr = pr.number;
    const header = document.createElement("div");
    header.className = "sp-pr-card-header";
    const link = document.createElement("a");
    link.href = pr.try;
    link.textContent = `#${pr.number} ${pr.title}`;
    link.title = "Its page as a try push, which imports it into this tree";
    header.append(link);
    if (pr.author) {
      header.append(` by ${pr.author}`);
    }
    if (pr.review) {
      const review = document.createElement("span");
      review.className = `spec-pr-review spec-pr-review-${pr.review_class}`;
      review.textContent = pr.review;
      header.append(" ", review);
    }
    if (pr.draft) {
      const draft = document.createElement("span");
      draft.className = "spec-pr-draft";
      draft.textContent = "Draft";
      header.append(" ", draft);
    }
    if (pr.url) {
      const github = document.createElement("a");
      github.className = "spec-pr-github";
      github.href = pr.url;
      github.textContent = "on GitHub";
      header.append(" ", github);
    }
    const note = document.createElement("div");
    note.className = "sp-pr-card-note";
    const [first, last] = change.trunk || [change.anchor, change.anchor];
    note.textContent = {
      change: `Changes ${first == last ? `line ${first}` : `lines ${first}-${last}`}:`,
      insert: `Adds, after line ${change.anchor}:`,
      remove: `Removes ${first == last ? `line ${first}` : `lines ${first}-${last}`}.`,
    }[change.kind];
    element.append(header, note);
    if (change.html) {
      const body = document.createElement("div");
      body.className = "sp-pr-card-body rendered-spec";
      body.innerHTML = change.html;
      element.append(body);
    }
    const changed = change.trunk ? blocksOfLines(first, last) : [];
    element.addEventListener("mouseenter", () => {
      for (const block of changed) {
        block.classList.add("sp-pr-changed");
      }
    });
    element.addEventListener("mouseleave", () => {
      for (const block of changed) {
        block.classList.remove("sp-pr-changed");
      }
    });
    column.append(element);
    return {
      element,
      header,
      pr: pr.number,
      block: blockOfLine(change.anchor),
      anchor: change.anchor,
      insert: change.kind == "insert" ? 1 : 0,
    };
  }

  async function load(entry) {
    const pr = JSON.parse(entry.dataset.pr);
    const state = entry.querySelector(".sp-pr-state");
    if (!pr.blocks) {
      return;
    }
    let data;
    try {
      data = await (await fetch(pr.blocks)).json();
    } catch (e) {
      state.textContent = "Its changes couldn't be loaded.";
      return;
    }
    entry.dataset.state = data.state;
    switch (data.state) {
      case "ok":
        state.textContent = data.changes.length
          ? `${plural(data.changes.length, "change")} on this page`
          : "No changes on this page";
        for (const change of data.changes) {
          const made = card(pr, change);
          made.element.hidden = entry.hidden;
          made.entry = entry;
          cards.push(made);
        }
        break;
      case "conflicts":
        state.textContent = "Its changes to this file conflict with the trunk's since its base.";
        break;
      case "removes-file":
        state.textContent = "It removes this file.";
        break;
      case "needs-import": {
        state.textContent = `${data.message} `;
        const link = document.createElement("a");
        link.href = pr.try;
        link.textContent = "Its page";
        state.append(link);
        break;
      }
      case "missing-head":
        state.textContent = "Its head hasn't been fetched yet.";
        break;
      default:
        state.textContent = data.message || data.state;
    }
  }

  // (The space before a block, as a row in tables.)
  function spaceBefore(block) {
    const row = block.tagName == "TR";
    const space = document.createElement(row ? "tr" : "div");
    space.className = "sp-pr-space";
    space.setAttribute("aria-hidden", "true");
    if (row) {
      space.append(document.createElement("td"));
    }
    block.before(space);
    return space;
  }

  function place() {
    for (const space of doc.querySelectorAll(".sp-pr-space")) {
      space.remove();
    }
    const shown = cards.filter(card => !card.element.hidden && card.block);
    const order = new Map(marks.map((mark, i) => [mark.block, i]));
    // (By their blocks, and lines, with lines added after a line after its
    // changes.)
    shown.sort((a, b) => (order.get(a.block) - order.get(b.block)) || (a.anchor - b.anchor) ||
      (a.insert - b.insert) || (a.pr - b.pr));
    const top0 = column.getBoundingClientRect().top;
    let bottom = null;
    let previous = null;
    const headers = new Map();
    let block = null;
    for (const card of shown) {
      const floor = bottom === null ? 0 : bottom + 6;
      let at = floor;
      if (card.block !== block) {
        block = card.block;
        let top = block.getBoundingClientRect().top;
        const goal = top0 + floor;
        if (goal > top + 1) {
          const space = spaceBefore(block);
          space.style.height = `${Math.round(goal - top)}px`;
          // (Less its margins' collapsing.)
          const moved = block.getBoundingClientRect().top;
          space.style.height = `${Math.max(0, Math.round(2 * goal - top - moved))}px`;
          top = block.getBoundingClientRect().top;
        }
        at = Math.max(top - top0, floor);
      }
      const lastHeader = headers.get(card.pr);
      const header = previous !== card.pr || lastHeader === undefined ||
        at - lastHeader > window.innerHeight;
      card.header.hidden = !header;
      if (header) {
        headers.set(card.pr, at);
      }
      card.element.style.top = `${at}px`;
      bottom = at + card.element.offsetHeight;
      previous = card.pr;
    }
    column.style.minHeight = `${bottom ?? 0}px`;
  }

  let scheduled = false;
  function schedule() {
    if (!scheduled) {
      scheduled = true;
      requestAnimationFrame(() => {
        scheduled = false;
        place();
      });
    }
  }

  // Filtering the entries hides their cards.
  new MutationObserver(() => {
    for (const card of cards) {
      card.element.hidden = card.entry.hidden;
    }
    schedule();
  }).observe(document.querySelector(".sp-pr-list"), {
    attributes: true,
    attributeFilter: ["hidden"],
    subtree: true,
  });
  let width = doc.clientWidth;
  new ResizeObserver(() => {
    if (doc.clientWidth != width) {
      width = doc.clientWidth;
      schedule();
    }
  }).observe(doc);
  document.fonts?.ready.then(schedule);

  Promise.all(entries.map(entry => load(entry).then(schedule))).then(() => {
    schedule();
    document.body.classList.add("sp-prs-loaded");
  });
})();
