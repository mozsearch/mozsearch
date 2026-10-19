"use strict";

// The subway map of a slicing's clusters (`slice-rev:` queries' default
// view; see tools/src/dataflow/subway.rs for the model, SUBWAY): the
// clusters' functions are cards of their stations (statement blocks), laid
// out by the calls between them with graph.js, and their values are colored
// lines: down each card's gutter (stopping, a dot, at the stations reading
// or writing them), and between cards along the calls (and returns, IPC
// messages and fields' writes to reads) they take, side by side.
//
// Colors: a family of lines (a root value and the values derived from it)
// has a hue, by its root's (semantic) type, so a principal's family is blue
// in every map; a value derived from another is its parent's hue's other
// variation (as blame's striping alternates), or for one of several derived
// from the same value (a split), a slightly rotated hue, and a value derived
// from several (a merge) is its family's hue again.
//
// A legend toggles lines, families and clusters, and cards (whose lines
// then go straight from their callers to their callees), and hovering a line
// there highlights it.  The dataflow graph (DOT) is a link away.
//
// Taint (see tools/src/dataflow/taint.rs): a line is dashed, with a red halo,
// where it's tainted (from where a parent actor receives it from a content
// process), until a validation (a green ring) blesses it.

var SubwayMap = new (class SubwayMap {
  constructor() {
    window.addEventListener("load", () => this.init(), { once: true });
  }

  // The hues of semantic types' families (see `subway::semantic_type`).
  static SEMANTIC_HUES = [
    [["nsIPrincipal", "BasePrincipal", "ContentPrincipal", "NullPrincipal", "SystemPrincipal",
      "ExpandedPrincipal", "PrincipalInfo", "ContentPrincipalInfo", "SystemPrincipalInfo",
      "NullPrincipalInfo", "ExpandedPrincipalInfo", "Principal"], 215],
    [["nsIURI", "URIParams", "nsIURL", "nsStandardURL", "MozURL"], 28],
    [["OriginAttributes", "OriginAttributesDictionary"], 285],
    [["nsACString", "nsCString", "nsAutoCString", "nsTSubstring", "nsTString", "nsAString",
      "nsString", "nsAutoString", "nsDependentCString", "nsDependentString", "char",
      "char16_t", "basic_string", "string"], 145],
    [["ClientInfo", "IPCClientInfo", "ClientState"], 175],
    [["_Bool", "bool"], 55],
    [["nsresult", "ErrorResult", "IPCResult", "Result"], 0],
  ];
  static TRACK = 9;

  init() {
    if (typeof SUBWAY === "undefined") {
      return;
    }
    document.documentElement.classList.add("for-interactive-graph");
    this.model = SUBWAY;
    this.tree = document.getElementById("data")?.dataset.tree;
    this.rev = new URLSearchParams(document.location.search).get("rev");
    this.hiddenLines = new Set();
    this.hiddenCards = new Set();
    this.colors = this.allocateColors();
    this.addStyles();
    this.buildLegend();
    if (!this.model.cards.length) {
      document.querySelector("#interactive-graph-container").textContent =
        "No functions to map (see the dataflow graph).";
      return;
    }
    this.graph = new Graph(
      document.querySelector("#interactive-graph-viewport"),
      document.querySelector("#interactive-graph-container"),
      this.input(), [], [], GRAPH_EXTRA,
      { renderBlock: (block, element) => this.renderCard(block, element) });
    document.documentElement.dataset.subwayReady = "true";
  }

  /** A semantic type's family hue (see SEMANTIC_HUES), else a hash's. */
  hueOf(semantic) {
    for (const [types, hue] of SubwayMap.SEMANTIC_HUES) {
      if (types.includes(semantic)) {
        return hue;
      }
    }
    let hash = 0;
    for (const c of semantic ?? "") {
      hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
    }
    return hash % 360;
  }

  /**
   * Each line's color: its family's hue (by its root's semantic type),
   * rotated for siblings derived from one value, and alternating between
   * two variations by generation; merges are their family's hue again.
   */
  allocateColors() {
    const lines = this.model.lines;
    const children = new Map(lines.map(line => [line.id, []]));
    for (const line of lines) {
      for (const parent of line.parents) {
        children.get(parent)?.push(line.id);
      }
    }
    const hue = new Map();
    const generation = new Map();
    const familyHues = new Map();
    for (const family of this.model.families) {
      familyHues.set(family.id, this.hueOf(family.semantic ?? family.label));
    }
    // (Breadth first from each family's roots; another root of a family is
    // a rotated hue of it.)
    const queue = [];
    for (const family of this.model.families) {
      const roots = lines.filter(line => line.family == family.id && !line.parents.length);
      roots.sort((a, b) => (a.id == family.root ? -1 : 0) - (b.id == family.root ? -1 : 0));
      roots.forEach((root, i) => {
        hue.set(root.id, familyHues.get(family.id) + i * 40);
        generation.set(root.id, 0);
        queue.push(root.id);
      });
    }
    while (queue.length) {
      const id = queue.shift();
      const kids = children.get(id);
      kids.forEach((kid, i) => {
        if (hue.has(kid)) {
          return;
        }
        const line = lines[kid];
        const merge = line.parents.length > 1;
        const rotation = kids.length > 1 ? (i % 2 ? 1 : -1) * 18 * Math.ceil((i + 1) / 2) : 0;
        hue.set(kid, merge ? familyHues.get(line.family) : hue.get(id) + rotation);
        generation.set(kid, generation.get(id) + 1);
        queue.push(kid);
      });
    }
    return new Map(lines.map(line => {
      const h = ((hue.get(line.id) ?? familyHues.get(line.family) ?? 0) % 360 + 360) % 360;
      const light = generation.get(line.id) % 2 ? 60 : 42;
      return [line.id, `hsl(${h}, 68%, ${light}%)`];
    }));
  }

  addStyles() {
    const style = document.createElement("style");
    style.textContent = this.model.lines.map(line =>
      `.interactive-graph-edge-line-${line.id}:not(.clicktarget),
       .interactive-graph-edge-line-${line.id}-tainted:not(.clicktarget) {
         stroke: ${this.colors.get(line.id)}; stroke-width: 3px; stroke-opacity: 1;
       }`
    ).join("\n") + `
      [class*="interactive-graph-edge-line-"][class$="-tainted"]:not(.clicktarget) {
        stroke-dasharray: 6 3;
        filter: drop-shadow(0 0 1.5px rgb(210, 0, 0));
      }`;
    document.head.append(style);
    this.focusStyle = document.createElement("style");
    document.head.append(this.focusStyle);
  }

  /** Highlight a line (or stop, with null). */
  focus(id) {
    this.focusStyle.textContent = id == null ? "" :
      `.interactive-graph-edge:not(.interactive-graph-edge-line-${id}):not(.interactive-graph-edge-line-${id}-tainted),
       .sw-track:not([data-line="${id}"]) { opacity: 0.15; }`;
  }

  /** A card's node: its function's symbol, else its id. */
  cardSym(card) {
    return card.sym ?? `card:${card.id}`;
  }

  sourceURL(path, line) {
    const base = this.rev ? `/${this.tree}/rev/${this.rev}` : `/${this.tree}/source`;
    return `${base}/${path}#${line}`;
  }

  /**
   * The graph: the shown cards, and an edge for each shown line of each
   * connector between them, by the line's own kind (so that graph.js lays
   * a connector's lines side by side); lines through hidden cards go
   * straight from where they come from to where they go.
   */
  input() {
    const cards = this.model.cards.filter(card => !this.hiddenCards.has(card.id));
    const edges = [];
    const seen = new Set();
    const add = (from, to, line, jump, tainted) => {
      const kind = `line-${line}` + (tainted ? "-tainted" : "");
      const key = `${from}->${to}:${kind}`;
      if (from == to || seen.has(key) || this.hiddenLines.has(line)) {
        return;
      }
      seen.add(key);
      edges.push({
        from: this.cardSym(this.model.cards[from]),
        to: this.cardSym(this.model.cards[to]),
        kind,
        jumps: jump ? [jump] : [],
        hovers: [],
      });
    };
    const connectors = this.model.connectors;
    for (const c of connectors) {
      for (const line of c.lines) {
        // (Through hidden cards: to where the line goes from them.)
        let targets = [c.to];
        const visited = new Set();
        while (targets.some(t => this.hiddenCards.has(t))) {
          targets = targets.flatMap(t => {
            if (!this.hiddenCards.has(t)) {
              return [t];
            }
            if (visited.has(t)) {
              return [];
            }
            visited.add(t);
            return connectors.filter(o => o.from == t && o.lines.includes(line)).map(o => o.to);
          });
        }
        if (this.hiddenCards.has(c.from)) {
          continue;
        }
        const from = this.model.cards[c.from];
        const station = c.from_station != null ? from.stations[c.from_station] : null;
        const jump = station?.rows.length ? `${from.path}#${station.rows[0].line}` : null;
        const tainted = c.tainted?.includes(line) ?? false;
        for (const to of targets) {
          add(c.from, to, line, jump, tainted);
        }
      }
    }
    return {
      nodes: cards.map(card => ({ sym: this.cardSym(card), pretty: `${card.func} #${card.id}`, card })),
      edges,
    };
  }

  /** The lines in a card, and where each is (first and last station). */
  cardTracks(card) {
    const spans = new Map();
    const extend = (line, station) => {
      if (this.hiddenLines.has(line)) {
        return;
      }
      const span = spans.get(line) ?? { first: station, last: station };
      span.first = Math.min(span.first, station);
      span.last = Math.max(span.last, station);
      spans.set(line, span);
    };
    card.stations.forEach((station, i) => station.stops.forEach(line => extend(line, i)));
    // (A taint source, a parameter, comes in at the top.)
    for (const source of this.model.taint?.sources ?? []) {
      if (source.card == card.id) {
        extend(source.line, -1);
      }
    }
    for (const c of this.model.connectors) {
      for (const line of c.lines) {
        // (Coming in at the top (before station 0), or for a return, at its
        // call's station; going out at the bottom (after the last station).)
        if (c.to == card.id) {
          extend(line, c.to_station ?? -1);
        }
        if (c.from == card.id) {
          extend(line, card.stations.length);
        }
      }
    }
    const order = [...spans.keys()].sort((a, b) =>
      this.model.lines[a].family - this.model.lines[b].family || a - b);
    return { card, order, spans };
  }

  /**
   * Whether a line is tainted arriving at a card's station `k` (after its
   * last station, for k past it: where it goes on from the card).
   */
  taintedBefore(card, line, k) {
    const station = card.stations[Math.max(0, k)];
    if (k >= card.stations.length || !station) {
      return card.tainted_out?.includes(line) ?? false;
    }
    return station.tainted?.includes(line) ?? false;
  }

  /** A row's gutter: its tracks, and (at its station's middle row) dots. */
  gutter(tracks, stationIndex, rowIndex, stopRow, stops) {
    const NS = "http://www.w3.org/2000/svg";
    const width = Math.max(1, tracks.order.length) * SubwayMap.TRACK;
    const svg = document.createElementNS(NS, "svg");
    svg.classList.add("sw-gutter");
    svg.setAttribute("width", width);
    svg.setAttribute("viewBox", `0 0 ${width} 10`);
    svg.setAttribute("preserveAspectRatio", "none");
    tracks.order.forEach((line, i) => {
      const span = tracks.spans.get(line);
      if (stationIndex < span.first || stationIndex > span.last) {
        return;
      }
      // (A line starts and ends at its stops' row, the middle of their
      // stations', unless it comes from above or goes on below.)
      let top = 0;
      let bottom = 10;
      if (stationIndex == span.first) {
        if (rowIndex < stopRow) {
          return;
        }
        if (rowIndex == stopRow) {
          top = 5;
        }
      }
      if (stationIndex == span.last) {
        if (rowIndex > stopRow) {
          return;
        }
        if (rowIndex == stopRow) {
          bottom = 5;
        }
      }
      if (top >= bottom) {
        return;
      }
      const x = i * SubwayMap.TRACK + SubwayMap.TRACK / 2;
      // (Tainted as it arrives at the station, above its stops' row, and as
      // it leaves, below.)
      const k = Math.ceil(stationIndex);
      const parts = [];
      if (Number.isInteger(stationIndex) && rowIndex >= stopRow) {
        if (rowIndex == stopRow && top < 5) {
          parts.push([top, 5, this.taintedBefore(tracks.card, line, k)]);
          top = 5;
        }
        parts.push([top, bottom, this.taintedBefore(tracks.card, line, k + 1)]);
      } else {
        parts.push([top, bottom, this.taintedBefore(tracks.card, line, k)]);
      }
      for (const [y1, y2, tainted] of parts) {
        if (y1 >= y2) {
          continue;
        }
        const segment = document.createElementNS(NS, "line");
        segment.setAttribute("x1", x);
        segment.setAttribute("x2", x);
        segment.setAttribute("y1", y1);
        segment.setAttribute("y2", y2);
        segment.setAttribute("stroke", this.colors.get(line));
        segment.classList.add("sw-track");
        segment.classList.toggle("sw-tainted", tainted);
        segment.dataset.line = line;
        svg.append(segment);
      }
    });
    return svg;
  }

  /**
   * A row's dots (the lines stopping at its station), over its gutter, and
   * its validations' rings (see `subway::Validation`).
   */
  dots(tracks, station, stationIndex) {
    const { stops, writes } = station;
    const validations = (this.model.taint?.validations ?? []).filter(v =>
      v.card == tracks.card.id && v.station == stationIndex);
    const box = document.createElement("span");
    box.className = "sw-dots";
    tracks.order.forEach((line, i) => {
      const checks = validations.filter(v => v.lines.includes(line));
      if (!stops.includes(line) && !checks.length) {
        return;
      }
      const dot = document.createElement("span");
      dot.className = "sw-dot sw-track" + (writes.includes(line) ? " sw-dot-write" : "");
      dot.dataset.line = line;
      dot.style.left = `${i * SubwayMap.TRACK + SubwayMap.TRACK / 2}px`;
      const label = this.model.lines[line].label;
      dot.title = `${label} ${writes.includes(line) ? "written" : "read"} here`;
      if (station.tainted?.includes(line)) {
        dot.classList.add("sw-dot-tainted");
        dot.title += " (tainted)";
      }
      if (checks.length) {
        const guards = checks.some(v => v.guards);
        dot.classList.add(guards ? "sw-dot-validate" : "sw-dot-check");
        dot.title += checks.map(v => guards ?
          `; validated by ${v.by} (blessing it, and what's derived from its sources, where that succeeded)` :
          `; checked by ${v.by}, but nothing depends on its success`).join("");
      }
      box.append(dot);
    });
    return box;
  }

  renderCard(block, element) {
    const card = block.data.card;
    const tracks = this.cardTracks(card);
    element.classList.add("sw-card");
    element.classList.toggle("sw-card-changed", card.changed);
    element.style.setProperty("--sw-gutter", `${Math.max(1, tracks.order.length) * SubwayMap.TRACK}px`);

    const head = document.createElement("div");
    head.className = "sw-head";
    const name = document.createElement("span");
    name.className = "sw-name";
    name.textContent = card.func;
    const where = document.createElement("a");
    where.className = "sw-where";
    const firstLine = card.stations[0]?.rows[0]?.line ?? 1;
    where.href = this.sourceURL(card.path, firstLine);
    where.textContent = `${card.path.split("/").pop()}:${firstLine}`;
    head.append(name, " ", where);
    element.append(head);

    card.stations.forEach((station, si) => {
      if (si > 0) {
        const gap = document.createElement("div");
        gap.className = "sw-row sw-gap";
        gap.append(this.gutter(tracks, si - 0.5, 0, -1, []));
        const text = document.createElement("span");
        text.className = "sw-gap-text";
        text.textContent = "⋮";
        gap.append(text);
        element.append(gap);
      }
      const stopRow = Math.floor((station.rows.length - 1) / 2);
      station.rows.forEach((row, ri) => {
        if (row.skipped) {
          const skip = document.createElement("div");
          skip.className = "sw-row sw-skip";
          skip.append(this.gutter(tracks, si, ri - 0.5, stopRow, []));
          const text = document.createElement("span");
          text.className = "sw-gap-text";
          text.textContent = `⋮ ${row.skipped} line${row.skipped == 1 ? "" : "s"}`;
          skip.append(text);
          element.append(skip);
        }
        const div = document.createElement("div");
        div.className = "sw-row" + (row.changed ? " sw-changed" : "");
        div.append(this.gutter(tracks, si, ri, stopRow, station.stops));
        if (ri == stopRow) {
          div.append(this.dots(tracks, station, si));
        }
        const number = document.createElement("a");
        number.className = "sw-lno";
        number.href = this.sourceURL(card.path, row.line);
        number.textContent = row.line;
        const code = document.createElement("code");
        code.className = "sw-code";
        code.textContent = row.text;
        div.append(number, code);
        element.append(div);
      });
    });
  }

  rerender() {
    this.graph?.setInput(this.input());
  }

  /** The legend: clusters, their families and lines; and the cards. */
  buildLegend() {
    const legend = document.querySelector("#sw-legend");
    if (!legend) {
      return;
    }
    legend.replaceChildren();
    const heading = document.createElement("h4");
    heading.textContent = "Values";
    const dot = document.createElement("a");
    dot.className = "sw-dot-link";
    const params = new URLSearchParams(document.location.search);
    params.set("q", `${params.get("q") ?? ""} slice-format:svg`);
    dot.href = `${document.location.pathname}?${params}`;
    dot.textContent = "Dataflow graph";
    dot.title = "The clusters' changes and the dataflow linking them, as a graph";
    legend.append(heading, dot);

    const checkbox = (checked, onchange) => {
      const input = document.createElement("input");
      input.type = "checkbox";
      input.checked = checked;
      input.addEventListener("change", () => onchange(input.checked));
      return input;
    };
    const lineItem = line => {
      const item = document.createElement("li");
      item.className = "sw-legend-line";
      item.dataset.line = line.id;
      const label = document.createElement("label");
      const box = checkbox(!this.hiddenLines.has(line.id), on => {
        on ? this.hiddenLines.delete(line.id) : this.hiddenLines.add(line.id);
        this.updateLegend();
        this.rerender();
      });
      box.dataset.line = line.id;
      const swatch = document.createElement("span");
      swatch.className = "sw-swatch";
      swatch.style.background = this.colors.get(line.id);
      swatch.classList.toggle("sw-swatch-tainted", line.tainted ?? false);
      const name = document.createElement("span");
      name.textContent = line.label;
      label.append(box, swatch, name);
      if (line.tainted) {
        const hazard = document.createElement("span");
        hazard.className = "sw-hazard";
        hazard.textContent = "\u2623";
        hazard.title = "Tainted where it's dashed: from a content process, until validated";
        label.append(" ", hazard);
      }
      if (line.type) {
        const type = document.createElement("span");
        type.className = "sw-type";
        type.textContent = line.type;
        label.append(" ", type);
      }
      label.title = line.names.join(", ");
      item.append(label);
      item.addEventListener("mouseenter", () => this.focus(line.id));
      item.addEventListener("mouseleave", () => this.focus(null));
      return item;
    };
    const familyItem = family => {
      const members = this.model.lines.filter(line => line.family == family.id);
      // (A family of one line is the line.)
      if (members.length == 1) {
        return lineItem(members[0]);
      }
      const item = document.createElement("li");
      item.className = "sw-legend-family";
      const label = document.createElement("label");
      const box = checkbox(true, on => {
        for (const line of members) {
          on ? this.hiddenLines.delete(line.id) : this.hiddenLines.add(line.id);
        }
        this.updateLegend();
        this.rerender();
      });
      box.dataset.family = family.id;
      label.append(box, family.label);
      if (family.semantic) {
        const type = document.createElement("span");
        type.className = "sw-type";
        type.textContent = family.semantic;
        label.append(" ", type);
      }
      item.append(label);
      const list = document.createElement("ul");
      list.append(...members.map(lineItem));
      item.append(list);
      return item;
    };
    for (const cluster of this.model.clusters) {
      const families = this.model.families.filter(family => family.cluster == cluster.n);
      if (!families.length) {
        continue;
      }
      const section = document.createElement("details");
      section.open = true;
      const summary = document.createElement("summary");
      summary.textContent = `Cluster ${cluster.n}` + (cluster.commits ? ` (${cluster.commits})` : "");
      const list = document.createElement("ul");
      list.className = "sw-legend-families";
      list.append(...families.map(familyItem));
      section.append(summary, list);
      legend.append(section);
    }

    const cards = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = "Functions";
    const list = document.createElement("ul");
    list.className = "sw-legend-cards";
    for (const card of this.model.cards) {
      const item = document.createElement("li");
      const label = document.createElement("label");
      const box = checkbox(true, on => {
        on ? this.hiddenCards.delete(card.id) : this.hiddenCards.add(card.id);
        this.rerender();
      });
      box.dataset.card = card.id;
      label.append(box, card.func);
      item.append(label);
      list.append(item);
    }
    cards.append(summary, list);
    legend.append(cards);

    if (this.model.taint) {
      const taint = document.createElement("details");
      taint.open = true;
      taint.className = "sw-legend-taint";
      const summary = document.createElement("summary");
      summary.textContent = "Taint";
      const list = document.createElement("ul");
      // (The unvalidated first, as an audit's list.)
      const ORDER = ["unvalidated", "partly", "validated", "unused"];
      const sources = [...this.model.taint.sources].sort((a, b) =>
        ORDER.indexOf(a.status) - ORDER.indexOf(b.status));
      for (const source of sources) {
        const item = document.createElement("li");
        const status = document.createElement("span");
        status.className = `sw-status sw-status-${source.status}`;
        status.textContent = source.status;
        status.title = `${source.tainted_stops} of its ${source.stops} stops tainted`;
        item.append(status, ` \u2623 ${this.model.lines[source.line].label}: ${source.why}`);
        item.dataset.line = source.line;
        item.addEventListener("mouseenter", () => this.focus(source.line));
        item.addEventListener("mouseleave", () => this.focus(null));
        list.append(item);
      }
      for (const v of this.model.taint.validations) {
        const item = document.createElement("li");
        const card = this.model.cards[v.card];
        const names = v.lines.map(line => this.model.lines[line].label).join(", ");
        item.textContent = v.guards ?
          `\u2713 ${card.func} validates ${names} (${v.by})` :
          `? ${card.func} calls ${v.by} on ${names}, but nothing depends on its success`;
        list.append(item);
      }
      if (!this.model.taint.validations.length) {
        const item = document.createElement("li");
        item.textContent = "No validations.";
        list.append(item);
      }
      taint.append(summary, list);
      legend.append(taint);
    }

    for (const note of this.model.notes) {
      const p = document.createElement("p");
      p.className = "sw-note";
      p.textContent = note;
      legend.append(p);
    }
  }

  /** The families' checkboxes' states, by their lines'. */
  updateLegend() {
    for (const box of document.querySelectorAll("#sw-legend input[data-family]")) {
      const members = this.model.lines.filter(line => line.family == box.dataset.family);
      const hidden = members.filter(line => this.hiddenLines.has(line.id)).length;
      box.checked = hidden < members.length;
      box.indeterminate = hidden > 0 && hidden < members.length;
    }
    for (const box of document.querySelectorAll("#sw-legend input[data-line]")) {
      box.checked = !this.hiddenLines.has(Number(box.dataset.line));
    }
  }
})();
