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
 * are (see side-cards.js).  A pull request whose first commits are another's
 * (ex: stacked on it) has only its own changes (its `blocks` URL's `after`;
 * see the tools' `spec_prs::containments`), unless the other's conflict with
 * the trunk's (`after_unused`), which its entry's note then says.  Each card has its pull request's header.  The
 * trunk's blocks with changes beside them are marked, and hovering over a
 * card outlines the ones
 * it changes.  Filtering the entries with the facets (see facets.js) hides
 * their cards.  The list is in the navigation panel, under the facets, and
 * its heading says how many entries the filtering shows; collapsing or
 * expanding it is remembered (see rendered_prs.liquid).  A card's "Hide"
 * hides its pull request (its cards and its entry) on the spec's pages,
 * whatever the filtering, until the panel's list of the hidden pull requests,
 * after the list, shows it again (see `setHidden`).
 *
 * An ecmarkup spec's page also lists the pull requests changing the files its
 * source imports, linking to those files' views with their changes (`imported`).
 *
 * Without the pull request's rendering (a Bikeshed spec's, until it's
 * imported, or when its changes conflict with the trunk's), its changes of
 * the source are cards of their removed and added lines instead, those beside
 * the same block in one card (only those of the page's lines, the article's
 * `data-rd-range`).
 *
 * The panel's "Changes" section (see change-nav.js) goes to the cards in
 * their order ("j" and "k"), from the one clicked, if any, which clicking
 * makes its current one.
 *
 * A card's rendering is styled as the page's blocks are, and what it changes
 * of the trunk's blocks is highlighted, from a diff of their words (see
 * `diffWords`): the words it adds in it (green), those the trunk has which
 * it removes in the trunk's blocks (red), and those replacing as many words
 * in both (yellow); the trunk's blocks a removal removes are red too, and an
 * addition's words are green: those it adds to the trunk's block, if it's
 * that block with words added (see `insertion`), or else all of them.  (With
 * CSS custom highlights, which leave the page's elements as they are.)  The
 * trunk's highlights are those of the cards whose blocks no other shown card
 * changes, and of the card hovered over or gone to (with "j" and "k").
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

  const [pageFirst, pageLast] = (doc.dataset.rdRange || "").split("-").map(Number);
  const onPage = change => !pageFirst || (change.anchor >= pageFirst && change.anchor <= pageLast);

  const plural = (n, noun) => `${n} ${noun}${n == 1 ? "" : "s"}`;
  const cards = [];

  // ## Diffs of the cards' renderings and the trunk's blocks

  // The text of `roots`' text nodes (but those of their blame strips and
  // spaces), and where each starts in it.
  function textOf(roots) {
    const nodes = [];
    let text = "";
    for (const root of roots) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
        acceptNode: node => node.parentElement?.closest(".blame-strip, .sp-pr-space")
          ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
      });
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        nodes.push({ node, start: text.length });
        text += node.data;
      }
      text += "\n";
    }
    return { text, nodes };
  }

  // A text's words, and its other characters but spaces one each, with where
  // they are.
  function wordsOf(text) {
    return [...text.matchAll(/[\p{L}\p{N}_]+|[^\s\p{L}\p{N}_]/gu)].map(m => ({
      text: m[0],
      start: m.index,
      end: m.index + m[0].length,
      word: /^[\p{L}\p{N}_]/u.test(m[0]),
    }));
  }

  // The most edits a diff finds (more, and it gives up; its time is the
  // items' times the edits, and its memory the edits squared).
  const MAX_EDITS = 1000;

  // Which of `a`'s and `b`'s items (strings) a shortest edit script of `a`
  // into `b` removes and adds (Myers' algorithm), as their indices' runs of
  // edits between equal items: [{ removed: [i...], added: [j...] }...]; or
  // null if they differ by more than MAX_EDITS.
  function diffItems(a, b) {
    const n = a.length;
    const m = b.length;
    const limit = Math.min(n + m, MAX_EDITS);
    const offset = limit + 1;
    const v = new Int32Array(2 * limit + 3);
    const trace = [];
    let found = -1;
    for (let d = 0; d <= limit && found < 0; d++) {
      // (The diagonals step d reads, -(d + 1) to d + 1.)
      trace.push(v.slice(offset - d - 1, offset + d + 2));
      for (let k = -d; k <= d; k += 2) {
        let x = (k == -d || (k != d && v[offset + k - 1] < v[offset + k + 1]))
          ? v[offset + k + 1] : v[offset + k - 1] + 1;
        let y = x - k;
        while (x < n && y < m && a[x] === b[y]) {
          x++;
          y++;
        }
        v[offset + k] = x;
        if (x >= n && y >= m) {
          found = d;
          break;
        }
      }
    }
    if (found < 0) {
      return null;
    }
    // (Back from the end: each step's equal items, then its edit, which
    // starts another run if equal items are between it and the next.)
    const runs = [];
    let run = null;
    let x = n;
    let y = m;
    for (let d = found; d >= 0; d--) {
      const before = trace[d];
      const at = k => before[k + d + 1];
      const k = x - y;
      const previous = (k == -d || (k != d && at(k - 1) < at(k + 1))) ? k + 1 : k - 1;
      const px = at(previous);
      const py = px - previous;
      while (x > px && y > py) {
        x--;
        y--;
        run = null;
      }
      if (d > 0) {
        if (!run) {
          run = { removed: [], added: [] };
          runs.push(run);
        }
        if (x == px) {
          run.added.unshift(py);
        } else {
          run.removed.unshift(px);
        }
      }
      x = px;
      y = py;
    }
    return runs.reverse();
  }

  // What `card` (a rendering) changes of the trunk's `blocks`: their words'
  // ranges which it removes, adds and changes ({ removed, added, changed },
  // with the trunk's and the card's sides of the changed), or null if their
  // diff is too big.
  function diffWords(blocks, body) {
    const trunk = textOf(blocks);
    const theirs = textOf([body]);
    const a = wordsOf(trunk.text);
    const b = wordsOf(theirs.text);
    const runs = diffItems(a.map(w => w.text), b.map(w => w.text));
    if (!runs) {
      return null;
    }
    // (A run only adding, or only removing, can be as far before as its
    // items equal those after it (ex: `speculationrules", "`, `"speculationrules",`
    // and `", "speculationrules`, added to a list): it's where it starts
    // after a line's start, or else a space, if it can, or else where it is.)
    const inRuns = [new Set(runs.flatMap(run => run.removed)), new Set(runs.flatMap(run => run.added))];
    const texts = [trunk.text, theirs.text];
    for (const run of runs) {
      for (const [side, items, words] of [[0, run.removed, a], [1, run.added, b]]) {
        if (!items.length || run[side ? "removed" : "added"].length) {
          continue;
        }
        // (Its items are consecutive, and the one before is kept.)
        const score = first => {
          const gap = texts[side].slice(first > 0 ? words[first - 1].end : 0, words[first].start);
          return gap.includes("\n") ? 2 : /\s/.test(gap) ? 1 : 0;
        };
        let first = items[0];
        let best = first;
        const n = items.length;
        while (first > 0 && !inRuns[side].has(first - 1) && words[first - 1].text == words[first + n - 1].text) {
          first--;
          if (score(first) > score(best)) {
            best = first;
          }
        }
        if (best != items[0]) {
          for (const item of items) {
            inRuns[side].delete(item);
          }
          items.splice(0, n, ...Array.from({ length: n }, (_, i) => best + i));
          for (const item of items) {
            inRuns[side].add(item);
          }
        }
      }
    }
    const result = { removed: [], added: [], trunkChanged: [], cardChanged: [] };
    for (const { removed, added } of runs) {
      // (A run replacing as many words with as many others changes them.)
      const changes = removed.length && removed.length == added.length &&
        removed.every(i => a[i].word) && added.every(j => b[j].word);
      result[changes ? "trunkChanged" : "removed"].push(...removed.map(i => a[i]));
      result[changes ? "cardChanged" : "added"].push(...added.map(j => b[j]));
    }
    return {
      removed: ranges(trunk, result.removed),
      trunkChanged: ranges(trunk, result.trunkChanged),
      added: ranges(theirs, result.added),
      cardChanged: ranges(theirs, result.cardChanged),
    };
  }

  // DOM ranges of `words` (in order) in `text` (see `textOf`), those only
  // spaces apart on a line as one.
  function ranges(text, words) {
    const spans = [];
    for (const word of words) {
      const last = spans[spans.length - 1];
      if (last && /^[^\S\n]*$/.test(text.text.slice(last.end, word.start))) {
        last.end = word.end;
      } else {
        spans.push({ start: word.start, end: word.end });
      }
    }
    // (The text node with an offset, and the offset in it.)
    const at = offset => {
      let low = 0;
      let high = text.nodes.length - 1;
      while (low < high) {
        const mid = (low + high + 1) >> 1;
        if (text.nodes[mid].start <= offset) {
          low = mid;
        } else {
          high = mid - 1;
        }
      }
      const { node, start } = text.nodes[low];
      return [node, Math.min(offset - start, node.data.length)];
    };
    return spans.map(({ start, end }) => {
      const range = new Range();
      range.setStart(...at(start));
      range.setEnd(...at(end));
      return range;
    }).filter(range => !range.collapsed);
  }

  // What an addition's rendering `body` adds: if it's the trunk's `block` (the
  // one at the line it adds after) with words added (ex: a value added to a
  // list in a paragraph, which is its source's added line), those words;
  // else (a block of its own) all of it.
  function insertion(block, body) {
    const diff = block ? diffWords([block], body) : null;
    if (diff && !diff.removed.length && !diff.trunkChanged.length && diff.added.length) {
      return diff;
    }
    return { removed: [], added: wholeRanges([body]), trunkChanged: [], cardChanged: [] };
  }

  // The whole text of `blocks`, as a range each.
  function wholeRanges(blocks) {
    return blocks.map(block => {
      const range = new Range();
      range.selectNodeContents(block);
      return range;
    });
  }

  const highlights = typeof CSS != "undefined" && CSS.highlights ? {
    removed: new Highlight(),
    added: new Highlight(),
    changed: new Highlight(),
  } : null;
  if (highlights) {
    CSS.highlights.set("sp-pr-removed", highlights.removed);
    CSS.highlights.set("sp-pr-added", highlights.added);
    CSS.highlights.set("sp-pr-changed-word", highlights.changed);
  }
  // (The card hovered over, and the one gone to.)
  let hovered = null;

  // Highlight the shown cards' changes, and those of the trunk's blocks of
  // the cards whose blocks no other shown card changes, or which are hovered
  // over or gone to.
  function highlight() {
    if (!highlights) {
      return;
    }
    for (const each of Object.values(highlights)) {
      each.clear();
    }
    const changers = new Map();
    for (const card of shown) {
      for (const block of card.marked) {
        changers.set(block, (changers.get(block) || 0) + 1);
      }
    }
    for (const card of shown) {
      if (!card.diff) {
        continue;
      }
      for (const range of card.diff.added) {
        highlights.added.add(range);
      }
      for (const range of card.diff.cardChanged) {
        highlights.changed.add(range);
      }
      const alone = card.marked.every(block => changers.get(block) == 1);
      if (alone || card === hovered || card.element.classList.contains("change-nav-current")) {
        for (const range of card.diff.removed) {
          highlights.removed.add(range);
        }
        for (const range of card.diff.trunkChanged) {
          highlights.changed.add(range);
        }
      }
    }
  }

  // A source change's lines, removed and added, with what changed within
  // the lines of a change replacing as many lines as it removes (between
  // their common start and end).
  function sourceLines(change) {
    const hunk = document.createElement("div");
    hunk.className = "sp-pr-hunk";
    const pairs = change.removed.length == change.added.length;
    const line = (className, text, other) => {
      const div = document.createElement("div");
      div.className = className;
      if (pairs && other !== undefined && other != text) {
        let start = 0;
        while (start < text.length && start < other.length && text[start] == other[start]) {
          start++;
        }
        let end = 0;
        while (end < text.length - start && end < other.length - start &&
               text[text.length - 1 - end] == other[other.length - 1 - end]) {
          end++;
        }
        div.append(text.slice(0, start));
        if (text.length - end > start) {
          const mark = document.createElement("mark");
          mark.textContent = text.slice(start, text.length - end);
          div.append(mark);
        }
        div.append(text.slice(text.length - end));
      } else {
        div.textContent = text;
      }
      hunk.append(div);
    };
    const more = n => {
      if (n) {
        const div = document.createElement("div");
        div.className = "sp-pr-more";
        div.textContent = `… ${plural(n, "more line")}`;
        hunk.append(div);
      }
    };
    change.removed.forEach((text, i) => line("sp-pr-del", text, change.added[i]));
    more(change.more_removed);
    change.added.forEach((text, i) => line("sp-pr-add", text, change.removed[i]));
    more(change.more_added);
    return hunk;
  }

  // A card for a change, or for source changes beside the same block.
  function card(pr, change, state) {
    const group = change.group || [change];
    const kind = group.every(each => each.kind == change.kind) ? change.kind : "change";
    const element = document.createElement("div");
    element.className = `sp-pr-card sp-pr-${kind}`;
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
    // (Hiding its pull request, on the spec's pages; see `setHidden`.)
    const hide = document.createElement("button");
    hide.type = "button";
    hide.className = "sp-pr-hide";
    hide.textContent = "Hide";
    hide.title = "Hide this pull request's changes on this spec's pages, whatever the filters " +
      "(the panel's list of hidden pull requests shows them again)";
    hide.addEventListener("click", () => setHidden(pr.number, pr.title, true));
    header.append(" ", hide);
    const note = document.createElement("div");
    note.className = "sp-pr-card-note";
    const lines = group.filter(each => each.trunk).map(each => each.trunk);
    const first = lines.length ? Math.min(...lines.map(([first]) => first)) : change.anchor;
    const last = lines.length ? Math.max(...lines.map(([, last]) => last)) : change.anchor;
    const range = first == last ? `line ${first}` : `lines ${first}-${last}`;
    const source = !!change.added;
    note.textContent = {
      change: `Changes ${range}`,
      insert: `Adds, after line ${change.anchor}`,
      remove: `Removes ${range}`,
    }[kind] + (!source ? (kind == "remove" ? "." : ":")
      : state == "conflicts" ? ", in its base's source, conflicting with the trunk's changes:"
      : ", in its source:");
    element.append(header, note);
    if (source) {
      element.classList.add("sp-pr-source-card");
      const body = document.createElement("div");
      body.className = "sp-pr-card-body sp-pr-source";
      body.append(...group.map(sourceLines));
      element.append(body);
    }
    let body = null;
    if (!source && change.html) {
      // (Styled as the page's blocks are; see .sp-pr-card-body.)
      body = document.createElement("div");
      body.className = "sp-pr-card-body rendered-doc rendered-spec";
      body.innerHTML = change.html;
      element.append(body);
    }
    const block = blockOfLine(change.anchor);
    const changed = [...new Set(lines.flatMap(([first, last]) => blocksOfLines(first, last)))];
    // (Those not in others, ex: a step's, not its note's too, for their text.)
    const outermost = changed.filter(block => !changed.some(other => other !== block && other.contains(block)));
    column.append(element);
    const made = {
      element,
      pr: pr.number,
      block,
      // (The blocks it changes, or the one it adds after.)
      marked: changed.length ? changed : [block].filter(Boolean),
      anchor: change.anchor,
      insert: kind == "insert" ? 1 : 0,
      // (What it changes of the trunk's blocks, highlighted; see
      // `highlight`.)
      diff: !highlights || source ? null
        : kind == "remove" ? { removed: wholeRanges(outermost), added: [], trunkChanged: [], cardChanged: [] }
        : kind == "change" && body && outermost.length ? diffWords(outermost, body)
        : kind == "insert" && body ? insertion(block, body)
        : null,
    };
    // (A change of the blocks' markup, but not of their text: ex: a table
    // cell's rowspan, for a row added below.)
    const diff = made.diff;
    if (kind == "change" && diff && !diff.added.length && !diff.removed.length &&
        !diff.trunkChanged.length && !diff.cardChanged.length) {
      note.textContent = note.textContent.replace(/:$/, " (its text is the same):");
    }
    element.addEventListener("mouseenter", () => {
      for (const block of changed) {
        block.classList.add("sp-pr-changed");
      }
      hovered = made;
      highlight();
    });
    // (Clicking it, but for its links or when selecting its text, makes it
    // the change the panel's "Changes" section is at.)
    element.addEventListener("click", event => {
      if (event.target.closest("a, button, summary, input") || String(window.getSelection())) {
        return;
      }
      document.dispatchEvent(new CustomEvent("change-nav-go", { detail: element }));
    });
    element.addEventListener("mouseleave", () => {
      for (const block of changed) {
        block.classList.remove("sp-pr-changed");
      }
      if (hovered === made) {
        hovered = null;
        highlight();
      }
    });
    return made;
  }

  function addCards(entry, pr, changes, state) {
    for (const change of changes) {
      const made = card(pr, change, state);
      made.entry = entry;
      made.element.hidden = !shownEntry(entry);
      cards.push(made);
    }
  }

  // ## Pull requests hidden by hand
  //
  // A card's "Hide" hides its pull request's cards and its entry on the
  // spec's pages (remembered, for the tree's spec repository), after the
  // facets' filtering, which doesn't show them again; the panel lists those
  // hidden, after the list of pull requests, each with a button showing it
  // again.
  const [, treeName, , specRepo] = location.pathname.split("/");
  const hiddenKey = storageKey(`spPrHidden/${treeName}/${specRepo}`);
  // (Its numbers and titles.)
  function readHidden() {
    try {
      const stored = JSON.parse(localStorage.getItem(hiddenKey) || "{}");
      return new Map(Object.entries(stored).map(([number, title]) => [Number(number), title]));
    } catch (e) {
      return new Map();
    }
  }
  let hidden = readHidden();
  const entryNumber = entry => Number(entry.dataset.path);
  const shownEntry = entry => !entry.hidden && !hidden.has(entryNumber(entry));

  function setHidden(number, title, hide) {
    // (As another of the spec's pages may have left them.)
    hidden = readHidden();
    if (hide) {
      hidden.set(number, title);
    } else if (number === null) {
      hidden.clear();
    } else {
      hidden.delete(number);
    }
    if (hidden.size) {
      localStorage.setItem(hiddenKey, JSON.stringify(Object.fromEntries(hidden)));
    } else {
      localStorage.removeItem(hiddenKey);
    }
    applyHidden();
  }

  function applyHidden() {
    for (const entry of entries) {
      entry.classList.toggle("sp-pr-entry-hidden", hidden.has(entryNumber(entry)));
    }
    for (const card of cards) {
      card.element.hidden = !shownEntry(card.entry);
    }
    updateCount();
    renderHidden();
    schedule();
  }

  // The panel's list of the hidden pull requests (those changing other pages
  // of the spec too, which it says), after the list of pull requests.
  let hiddenBox = null;
  function renderHidden() {
    if (!hidden.size) {
      hiddenBox?.remove();
      hiddenBox = null;
      return;
    }
    if (!hiddenBox) {
      hiddenBox = document.createElement("details");
      hiddenBox.className = "sp-pr-hidden-box";
      hiddenBox.open = true;
      (document.querySelector(".sp-pr-list-box") || entries[0].closest("ul")).after(hiddenBox);
    }
    const summary = document.createElement("summary");
    const heading = document.createElement("h4");
    const count = document.createElement("span");
    count.className = "sp-pr-hidden-count";
    count.textContent = `(${hidden.size})`;
    heading.append("Hidden pull requests ", count);
    summary.append(heading);
    const note = document.createElement("p");
    note.className = "sp-pr-hidden-note";
    note.textContent = "Hidden on this spec's pages, whatever the filters.";
    const list = document.createElement("ul");
    list.className = "facet-items sp-pr-hidden-list";
    const onPage = new Set(entries.map(entryNumber));
    for (const [number, title] of [...hidden].sort((a, b) => b[0] - a[0])) {
      const item = document.createElement("li");
      item.className = "sp-pr-hidden-entry";
      item.dataset.pr = number;
      const show = document.createElement("button");
      show.type = "button";
      show.className = "sp-pr-unhide";
      show.textContent = "Show";
      show.title = "Show its changes again";
      show.addEventListener("click", () => setHidden(number, title, false));
      item.append(`#${number} ${title}`, onPage.has(number) ? "" : " (not on this page)", " ", show);
      list.append(item);
    }
    const parts = [summary, note, list];
    if (hidden.size > 1) {
      const all = document.createElement("button");
      all.type = "button";
      all.className = "sp-pr-unhide sp-pr-unhide-all";
      all.textContent = "Show them all";
      all.addEventListener("click", () => setHidden(null, null, false));
      parts.push(all);
    }
    const open = hiddenBox.open;
    hiddenBox.replaceChildren(...parts);
    hiddenBox.open = open;
  }

  // Source changes beside the same block, as one.
  function grouped(changes) {
    const groups = [];
    let block;
    for (const change of changes.filter(onPage)) {
      const at = blockOfLine(change.anchor);
      const last = groups[groups.length - 1];
      if (last && at === block) {
        last.group.push(change);
      } else {
        groups.push({ ...change, group: [change] });
      }
      block = at;
    }
    return groups;
  }

  // A note linking to the views of the files the page's source imports which
  // a pull request changes.
  function importedNote(pr, only) {
    const note = document.createElement("span");
    note.className = "sp-pr-imported";
    note.append(only ? "It changes the files this page imports: " : " It also changes the files this page imports: ");
    pr.imported.forEach(({ path, view }, i) => {
      const link = document.createElement("a");
      link.href = view;
      link.textContent = path.split("/").pop();
      link.title = `${path}, with the pull requests' changes beside it`;
      note.append(i ? ", " : "", link);
    });
    note.append(".");
    return note;
  }

  async function load(entry) {
    const pr = JSON.parse(entry.dataset.pr);
    const state = entry.querySelector(".sp-pr-state");
    if (!pr.blocks) {
      if (pr.imported?.length) {
        state.replaceChildren(importedNote(pr, true));
      }
      return;
    }
    try {
      await loadBlocks(entry, pr, state);
    } finally {
      if (pr.imported?.length) {
        state.append(importedNote(pr, false));
      }
    }
  }

  async function loadBlocks(entry, pr, state) {
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
        addCards(entry, pr, data.changes, data.state);
        break;
      case "conflicts": {
        const changes = grouped(data.changes || []);
        state.textContent = "Its changes to this file conflict with the trunk's since its base" +
          (changes.length ? `; beside the page, its ${plural(changes.length, "change")} of its base's source.` : ".");
        addCards(entry, pr, changes, data.state);
        break;
      }
      case "removes-file":
        state.textContent = "It removes this file.";
        break;
      case "needs-import": {
        const changes = grouped(data.changes || []);
        state.textContent = changes.length
          ? `${plural(changes.length, "change")} on this page, in its source. ${data.message} `
          : `${data.message} `;
        const link = document.createElement("a");
        link.href = pr.try;
        link.textContent = "Its page";
        state.append(link);
        addCards(entry, pr, changes, data.state);
        break;
      }
      case "missing-head":
        state.textContent = "Its head hasn't been fetched yet.";
        break;
      default:
        state.textContent = data.message || data.state;
    }
    // (Its first commits are another's, whose changes conflict with the
    // trunk's, so its changes here are all of them; see the tools'
    // `spec_pr_blocks_json`.)
    const note = entry.querySelector(".sp-pr-contains");
    const tail = note?.lastChild;
    const own = "; only its own changes are shown here.";
    if (data.after_unused && tail?.nodeType == Node.TEXT_NODE && tail.data.endsWith(own)) {
      tail.data = tail.data.slice(0, -own.length) +
        "; its changes here include theirs, which conflict with the trunk's.";
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

  // The shown cards, in their order.
  let shown = [];

  function place() {
    for (const block of doc.querySelectorAll(".sp-pr-marked")) {
      block.classList.remove("sp-pr-marked");
    }
    shown = cards.filter(card => !card.element.hidden && card.block);
    const order = new Map(marks.map((mark, i) => [mark.block, i]));
    // (By their blocks, and lines, with lines added after a line after its
    // changes.)
    shown.sort((a, b) => (order.get(a.block) - order.get(b.block)) || (a.anchor - b.anchor) ||
      (a.insert - b.insert) || (a.pr - b.pr));
    for (const card of shown) {
      for (const block of card.marked) {
        block.classList.add("sp-pr-marked");
      }
    }
    placeSideCards({
      doc,
      columns: [{ aside: column, cards: shown }],
      blocks: [...new Set(shown.map(card => card.block))],
      spaceClass: "sp-pr-space",
      spaceBefore,
      gap: () => 6,
    });
    highlight();
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

  // The list's heading's count of the entries shown, and its collapsing or
  // expanding, remembered when it's the reader's.
  const listBox = document.querySelector(".sp-pr-list-box");
  const listCount = listBox?.querySelector(".sp-pr-list-count");
  const updateCount = () => {
    if (listCount) {
      const shown = entries.filter(shownEntry).length;
      listCount.textContent = shown == entries.length
        ? `(${entries.length})` : `(${shown} of ${entries.length})`;
    }
  };
  updateCount();
  let toggling = false;
  listBox?.querySelector("summary").addEventListener("click", () => {
    toggling = true;
  });
  listBox?.addEventListener("toggle", () => {
    if (toggling) {
      toggling = false;
      localStorage.setItem(storageKey("spPrListOpen"), listBox.open);
    }
  });

  // Filtering the entries hides their cards (and those hidden by hand stay
  // hidden).
  new MutationObserver(applyHidden).observe(document.querySelector(".sp-pr-list"), {
    attributes: true,
    attributeFilter: ["hidden"],
    subtree: true,
  });
  // (Those hidden on the spec's other pages are on this one too.)
  applyHidden();
  // (The card gone to with "j" and "k" has its trunk's highlights.)
  new MutationObserver(highlight).observe(column, {
    attributes: true,
    attributeFilter: ["class"],
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

  // (A few at a time: a big spec's page can have hundreds of pull requests,
  // each rendering the page with its changes; see `spec_pr_blocks_json`.)
  const queue = [...entries];
  const worker = async () => {
    while (queue.length) {
      try {
        await load(queue.shift());
      } catch (e) {
        console.error(e);
      }
      schedule();
    }
  };
  Promise.all(Array.from({ length: 6 }, worker)).then(() => {
    place();
    document.body.classList.add("sp-prs-loaded");
    if (cards.length) {
      // (For change-nav.js: the shown cards, and the blocks they mark.)
      document.dispatchEvent(new CustomEvent("change-nav-targets", {
        detail: () => shown.map(card => ({ element: card.element, marked: card.marked })),
      }));
    }
  });
})();
