/**
 * The excerpts of the definitions a stack's page (an explore page, or a try
 * push's) shows as changed under its files (see explore_file.liquid's outline
 * rows), in the navigation panel: the excerpt of the row under the mouse, or
 * of the row clicked, which stays until a click elsewhere or Escape.
 *
 * Each file's excerpts' data (`data-excerpts`; see
 * `format::outline_excerpts_json`) has its definitions' changed lines and the
 * lines around them, which the excerpts choose from by independent scores:
 * the definition's own changes (added lines, and removed ones) are always
 * shown, and the other lines by the most they matter as the structure around
 * the changes (ex: the condition of the `if` a change is in; see
 * `format::excerpt_structure`), by their nearness to changes, and as the
 * definition's first line, as many as fit in the panel's room (the fancy
 * bar's height less its other sections'), with "⋮ N lines" for the lines
 * between them.  Each line has the stack strip of the commits which changed
 * it (see stack-strip.js), numbered as the page's commits are.
 */
(function () {
  const panelContent = document.getElementById("panel-content");
  if (!panelContent || !document.querySelector(".explore-file[data-excerpts]")) {
    return;
  }

  // The fewest lines an excerpt shows, and how many it shows when the
  // panel isn't the fancy bar, whose room is the window's.
  const MIN_ROWS = 12;
  const FLOATING_ROWS = 30;
  // How much a line next to a change (1 line away, 2, 3) matters.
  const NEARNESS = [0, 0.7, 0.45, 0.3];
  // How much a changed line which isn't the definition's own (ex: a nested
  // definition's) matters, the start of a statement a change continues, and
  // the definition's last line.
  const OTHER_CHANGE = 0.5;
  const CHANGED_STATEMENT = 0.85;
  const LAST_LINE = 0.2;
  const FIRST_LINE = 0.95;

  const heading = document.createElement("h4");
  heading.textContent = "Changes";
  const section = document.createElement("div");
  section.id = "panel-excerpt";
  section.className = "excerpt-section";
  const hint = document.createElement("p");
  hint.className = "excerpt-hint";
  hint.textContent = "Hover over a changed definition to see its changes here, or click it to keep them here.";
  section.append(hint);
  // (After the facets.)
  panelContent.append(heading, section);

  const fetches = new Map();
  function excerptsData(url) {
    if (!fetches.has(url)) {
      fetches.set(url, fetch(url).then(response => {
        if (!response.ok) {
          throw new Error(`${response.status}`);
        }
        return response.json();
      }));
    }
    return fetches.get(url);
  }

  // The row whose excerpt is shown, whether it's kept (clicked), and the
  // latest request's number, so that slower earlier ones don't replace it.
  let shown = null;
  let sticky = false;
  let request = 0;

  async function show(row) {
    if (row == shown) {
      return;
    }
    shown?.classList.remove("excerpt-shown");
    shown = row;
    row.classList.add("excerpt-shown");
    const file = row.closest(".explore-file");
    const number = ++request;
    section.classList.add("excerpt-loading");
    try {
      const data = await excerptsData(file.dataset.excerpts);
      if (number == request) {
        render(row, data);
      }
    } catch (e) {
      if (number == request) {
        section.replaceChildren(note(`The excerpt couldn't be loaded (${e.message}).`));
      }
    } finally {
      if (number == request) {
        section.classList.remove("excerpt-loading");
      }
    }
  }

  function note(text) {
    const p = document.createElement("p");
    p.className = "excerpt-hint";
    p.textContent = text;
    return p;
  }

  function stick(row) {
    document.querySelector(".excerpt-selected")?.classList.remove("excerpt-selected");
    sticky = true;
    row.classList.add("excerpt-selected");
    show(row);
  }

  function unstick() {
    sticky = false;
    document.querySelector(".excerpt-selected")?.classList.remove("excerpt-selected");
  }

  // The candidate rows of a definition's excerpt, in order: the lines of
  // the "to" file in its lines (that the data has), with the lines removed
  // between them, or for a removed definition, its removed lines, and for
  // the changes outside of any definition, the lines around them.
  function candidates(data, key) {
    const entry = data.entries[key];
    if (!entry) {
      return [];
    }
    const own = new Set(entry.own);
    const removedAfter = new Map();
    for (const index of entry.removed) {
      const removed = data.removed[index];
      if (!removedAfter.has(removed.after)) {
        removedAfter.set(removed.after, []);
      }
      removedAfter.get(removed.after).push(removed);
    }
    const removedRow = removed => ({
      removed: true, line: removed.line, html: removed.h, own: true, s: 0,
    });
    if (key.startsWith("from:")) {
      return entry.removed.map(index => removedRow(data.removed[index]));
    }
    let lines;
    if (key == "top") {
      const near = new Set();
      for (const line of [...entry.own, ...entry.removed.map(index => data.removed[index].after)]) {
        for (let n = line - 3; n <= line + 3; n++) {
          near.add(n);
        }
      }
      lines = [...near].filter(n => data.lines[n]).sort((a, b) => a - b);
    } else {
      const [first, last] = key.split("-").map(Number);
      lines = Object.keys(data.lines).map(Number)
        .filter(n => n >= first && n <= last).sort((a, b) => a - b);
    }
    const rows = [];
    const addRemoved = after => {
      for (const removed of removedAfter.get(after) || []) {
        rows.push(removedRow(removed));
      }
      removedAfter.delete(after);
    };
    for (const n of lines) {
      addRemoved(n - 1);
      const line = data.lines[n];
      rows.push({
        line: n, html: line.h, changed: !!line.c, own: own.has(n), s: line.s || 0, u: line.u,
      });
    }
    for (const after of [...removedAfter.keys()].sort((a, b) => a - b)) {
      addRemoved(after);
    }
    return rows;
  }

  // Which rows to show in `budget` rows (counting "⋮ N lines" gaps as rows):
  // the own changes (or as many as fit, leaving a row to say how many
  // didn't), then the others by how much they matter, each with the earlier
  // lines of the statement it continues (`u`; see
  // `format::excerpt_statements`), if they fit, rather than a fragment.
  function choose(rows, key, budget) {
    const shown = new Set();
    const index = new Map(rows.map((row, i) => [row.removed ? `-${row.line}` : row.line, i]));
    const statement = i => {
      const start = rows[i].u && index.get(rows[i].u);
      return start === undefined || start > i ? [i] : [...Array(i - start + 1).keys()].map(k => start + k);
    };
    const own = rows.map((row, i) => (row.own ? i : -1)).filter(i => i >= 0);
    const cost = set => {
      let count = set.size;
      let previous = -1;
      for (const i of [...set].sort((a, b) => a - b)) {
        if (i > previous + 1) {
          count++;
        }
        previous = i;
      }
      return count + (previous < rows.length - 1 ? 1 : 0);
    };
    for (const i of own) {
      shown.add(i);
      if (cost(shown) > budget - 1) {
        shown.delete(i);
        return { shown, hidden: own.length - shown.size };
      }
    }
    const distance = rows.map(() => Infinity);
    for (const i of own) {
      for (let d = 1; d < NEARNESS.length; d++) {
        for (const j of [i - d, i + d]) {
          if (j >= 0 && j < rows.length) {
            distance[j] = Math.min(distance[j], d);
          }
        }
      }
    }
    const isDefinition = key != "top" && !key.startsWith("from:");
    const changedStatements = new Set(own.flatMap(i => statement(i)));
    const scored = rows.map((row, i) => {
      const scores = [row.s, NEARNESS[distance[i]] || 0];
      if (changedStatements.has(i)) {
        scores.push(CHANGED_STATEMENT);
      }
      if (row.changed) {
        scores.push(OTHER_CHANGE);
      }
      if (isDefinition && i == 0) {
        scores.push(FIRST_LINE);
      }
      if (isDefinition && i == rows.length - 1) {
        scores.push(LAST_LINE);
      }
      return { i, score: Math.max(...scores), distance: distance[i] };
    }).filter(({ i, score }) => !shown.has(i) && score > 0);
    scored.sort((a, b) => b.score - a.score || a.distance - b.distance || a.i - b.i);
    for (const { i } of scored) {
      const group = statement(i).filter(j => !shown.has(j));
      group.forEach(j => shown.add(j));
      if (cost(shown) > budget) {
        group.forEach(j => shown.delete(j));
      }
    }
    // (A gap of one line is shown as the line, which takes the same room.)
    for (let i = 1; i < rows.length - 1; i++) {
      if (!shown.has(i) && shown.has(i - 1) && shown.has(i + 1)) {
        shown.add(i);
      }
    }
    return { shown, hidden: 0 };
  }

  // How many rows fit in the panel's room, given a row's height.
  function budget(rowHeight, headerHeight) {
    const panel = document.getElementById("panel");
    if (!document.documentElement.classList.contains("fancy-bar-open") || !rowHeight) {
      return FLOATING_ROWS;
    }
    let used = document.getElementById("panel-toggle")?.getBoundingClientRect().height || 0;
    for (const child of panelContent.children) {
      if (child != section) {
        used += child.getBoundingClientRect().height;
      }
    }
    const room = panel.clientHeight - used - headerHeight - 16;
    return Math.max(MIN_ROWS, Math.floor(room / rowHeight));
  }

  // The stack strip's slots of each line of the "to" file (1-based), and its
  // scars (see stack-strip.js).
  function stripSlots(strip) {
    const slots = new Map();
    const scars = new Map();
    for (const [line, lineSlots] of strip?.lines || []) {
      slots.set(line + 1, lineSlots);
    }
    for (const [line, lineSlots] of strip?.scars || []) {
      scars.set(line + 1, lineSlots);
    }
    return { slots, scars };
  }

  function stripCell(strip, slots, scars, slot) {
    const cell = document.createElement("span");
    cell.className = "excerpt-strip";
    cell.style.width = `${strip.commits.length * slot}px`;
    const add = (k, scar) => {
      const span = document.createElement("span");
      span.className = scar ? "excerpt-slot excerpt-scar" : "excerpt-slot";
      span.dataset.slot = k;
      span.style.left = `${k * slot}px`;
      span.style.width = `${slot}px`;
      span.title = `${scar ? "Removed tokens here" : "Changed this line"}: ${k + 1}. ${strip.commits[k]?.[1] || ""}`;
      cell.append(span);
    };
    for (const k of slots || []) {
      add(k, false);
    }
    for (const k of scars || []) {
      add(k, true);
    }
    return cell;
  }

  function render(row, data) {
    const key = row.dataset.range;
    const rows = candidates(data, key);
    const header = document.createElement("div");
    header.className = "excerpt-header";
    const name = document.createElement("span");
    name.className = "excerpt-name";
    name.textContent = row.querySelector(".explore-outline-name")?.textContent.trim() || "";
    header.append(name);
    const badge = row.querySelector(".outline-badge");
    if (badge) {
      header.append(badge.cloneNode(true));
    }
    const path = document.createElement("div");
    path.className = "excerpt-path";
    path.textContent = data.path;
    header.append(path);

    const body = document.createElement("div");
    body.className = "excerpt-rows";
    section.replaceChildren(header, body);
    if (!rows.length) {
      body.append(note("Its changes aren't in the file's lines (ex: only moved)."));
      return;
    }

    const strip = data.strip?.commits?.length ? data.strip : null;
    const { slots, scars } = stripSlots(strip);
    const slot = strip && strip.commits.length <= 8 ? 4 : strip && strip.commits.length <= 24 ? 3 : 2;
    const makeRow = candidate => {
      const div = document.createElement("div");
      div.className = "excerpt-row";
      if (candidate.removed) {
        div.classList.add("excerpt-removed");
      } else if (candidate.changed) {
        div.classList.add("excerpt-added");
      }
      if (candidate.s && !candidate.changed && !candidate.removed) {
        div.classList.add("excerpt-structure");
        div.title = "Shown as the structure around the changes";
      }
      if (strip) {
        div.append(candidate.removed
          ? stripCell(strip, [], [], slot)
          : stripCell(strip, slots.get(candidate.line), scars.get(candidate.line), slot));
      }
      const lineno = document.createElement("a");
      lineno.className = "excerpt-lineno";
      lineno.href = `${candidate.removed ? data.from_link : data.link}#${candidate.line}`;
      lineno.textContent = candidate.removed ? "−" : candidate.line;
      if (candidate.removed) {
        lineno.title = `Line ${candidate.line} before the changes`;
      }
      const code = document.createElement("code");
      code.className = "source-line";
      code.innerHTML = candidate.html || " ";
      div.append(lineno, code);
      return div;
    };
    const gap = count => {
      const div = document.createElement("div");
      div.className = "excerpt-gap";
      div.textContent = `⋮ ${count} line${count == 1 ? "" : "s"}`;
      return div;
    };

    // (The room, from a row's height, measured on the first row.)
    const probe = makeRow(rows[0]);
    body.append(probe);
    const rowHeight = probe.getBoundingClientRect().height;
    probe.remove();
    const { shown: chosen, hidden } = choose(rows, key, budget(rowHeight, header.getBoundingClientRect().height));
    let previous = -1;
    for (const i of [...chosen].sort((a, b) => a - b)) {
      if (i > previous + 1) {
        body.append(gap(i - previous - 1));
      }
      body.append(makeRow(rows[i]));
      previous = i;
    }
    if (previous < rows.length - 1) {
      body.append(gap(rows.length - 1 - previous));
    }
    if (hidden) {
      body.append(note(`And ${hidden} more changed line${hidden == 1 ? "" : "s"}.`));
    }
  }

  document.addEventListener("mouseover", event => {
    if (sticky) {
      return;
    }
    const row = event.target.closest(".explore-file[data-excerpts] tr[data-range]");
    if (row) {
      show(row);
    }
  });
  document.addEventListener("click", event => {
    const row = event.target.closest(".explore-file[data-excerpts] tr[data-range]");
    if (row && !event.target.closest("a")) {
      if (sticky && shown == row) {
        unstick();
      } else {
        stick(row);
      }
    } else if (!row && !event.target.closest("#panel")) {
      unstick();
    }
  });
  document.addEventListener("keydown", event => {
    if (event.key == "Escape" && sticky) {
      unstick();
    }
  });
  // (The stack strip's slots of a commit light up together.)
  section.addEventListener("mouseover", event => {
    const k = event.target.closest(".excerpt-slot")?.dataset.slot;
    for (const slot of section.querySelectorAll(".excerpt-slot")) {
      slot.classList.toggle("excerpt-slot-hot", slot.dataset.slot === k);
    }
  });
  // (The panel's room changes, ex: as the window or the facets do.)
  let rendered = null;
  new ResizeObserver(() => {
    const row = shown;
    if (row && rendered != document.getElementById("panel").clientHeight) {
      rendered = document.getElementById("panel").clientHeight;
      shown = null;
      show(row);
    }
  }).observe(document.getElementById("panel"));
})();
