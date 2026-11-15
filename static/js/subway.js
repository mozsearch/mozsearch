"use strict";

// The subway map of a slicing's clusters (`slice-rev:` queries' default
// view; see tools/src/dataflow/subway.rs for the model, SUBWAY), or of a
// trace (`follow-forward:` and `follow-backward:` queries', whose steps' rows
// are highlighted, and whose seed's card is marked): the
// clusters' functions are cards of their stations (statement blocks), laid
// out by the calls between them with graph.js, and their values are colored
// lines: down each card's gutter (stopping, a dot, at the rows of the
// stations reading or writing them), and between cards along the calls (and
// returns, IPC messages and fields' writes to reads) they take, side by side.
//
// Colors: a family of lines (a root value and the values derived from it)
// has a hue, by its root's (semantic) type, so a principal's family is blue
// in every map, and its lines are shades of it, each its own (hues around
// the family's, in alternating lightness, roots and parents first), so that
// a family's lines can be told apart (a family can have dozens of lines, ex:
// what's derived from a message's data; when merges, values derived from
// several, all had the family's own hue, many of them looked the same).
//
// A legend toggles lines, families and clusters, and cards (whose lines
// then go straight from their callers to their callees), and hovering a line
// there, or its track or dots in a card, highlights it, and its symbols'
// tokens in the cards' code (which is highlighted as source listings are,
// with their context menus; see `subway::add_rows_html`), and hovering a
// station highlights its rows, its lines, and their tokens.  The dataflow
// graph (DOT) is a link away.
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
    // (The cards' code's symbols' context menus' SYM_INFO.)
    window.SYM_INFO = Object.assign(window.SYM_INFO || {}, this.model.sym_info || {});
    this.tree = document.getElementById("data")?.dataset.tree;
    this.rev = new URLSearchParams(document.location.search).get("rev");
    this.hiddenLines = new Set();
    this.hiddenCards = new Set();
    // (The values' symbols, which a macro's argument has if it's traced.)
    this.valueSyms = new Set(this.model.lines.flatMap(line => line.syms ?? []));
    for (const card of this.model.cards) {
      this.expandComposites(card);
    }
    this.colors = this.allocateColors();
    this.addStyles();
    this.buildLegend();
    if (!this.model.cards.length) {
      document.querySelector("#interactive-graph-container").textContent =
        "No functions to map (see the dataflow graph).";
      return;
    }
    // (In rows, whose edges end at their lines' tracks in the cards, so that
    // each line is one through them and between them; see `afterLayout`.
    // With `?layout=grid`, in graph.js's grid, whose edges end where its
    // ports are.)
    this.graph = new Graph(
      document.querySelector("#interactive-graph-viewport"),
      document.querySelector("#interactive-graph-container"),
      this.input(), [], [], GRAPH_EXTRA,
      {
        renderBlock: (block, element) => this.renderCard(block, element),
        layout: new URLSearchParams(document.location.search).get("layout") || "rows",
        afterLayout: graph => this.afterLayout(graph),
        // (A forward trace's seed's card first, as its values go from it,
        // so that the edges go down from it, but for those back up, ex: its
        // callees' returns.)
        isRoot: block => this.model.trace?.dir == "forward" && (block.data?.card?.seed ?? false),
      });
    this.addHovering(document.querySelector("#interactive-graph-container"));
    document.documentElement.dataset.subwayReady = "true";
  }

  /**
   * Hovering over a card's track or dot highlights its line, and over a
   * station, the station's rows and the lines stopping there, with their
   * symbols' tokens in the cards' code (see `brush`).
   */
  addHovering(container) {
    let hovered = null;
    container.addEventListener("mouseover", event => {
      let lines = null;
      let station = null;
      const dot = event.target.closest(".sw-dot");
      const gutter = event.target.closest(".sw-gutter");
      if (dot) {
        lines = [Number(dot.dataset.line)];
      } else if (gutter?.dataset.order) {
        const order = gutter.dataset.order.split(",").map(Number);
        const x = event.clientX - gutter.getBoundingClientRect().left;
        const line = order[Math.floor(x / (gutter.getBoundingClientRect().width / order.length))];
        lines = line === undefined ? null : [line];
      }
      if (!lines) {
        station = event.target.closest(".sw-station");
        lines = station ? station.dataset.stops.split(",").filter(Boolean).map(Number) : null;
      }
      const key = `${station?.dataset.key ?? ""}:${lines?.join(",") ?? ""}`;
      if (key == hovered) {
        return;
      }
      hovered = key;
      container.querySelector(".sw-station-hot")?.classList.remove("sw-station-hot");
      station?.classList.add("sw-station-hot");
      this.focus(lines);
      this.brush(container, lines);
    });
    container.addEventListener("mouseleave", () => {
      hovered = null;
      container.querySelector(".sw-station-hot")?.classList.remove("sw-station-hot");
      this.focus(null);
      this.brush(container, null);
    });
  }

  /** Highlight the tokens of lines' symbols (see `Line::syms`) in the cards. */
  brush(container, lines) {
    for (const token of container.querySelectorAll(".sw-brushed")) {
      token.classList.remove("sw-brushed");
    }
    const syms = new Set((lines ?? []).flatMap(line => this.model.lines[line]?.syms ?? []));
    if (!syms.size) {
      return;
    }
    for (const token of container.querySelectorAll(".sw-code [data-symbols]")) {
      if (token.dataset.symbols.split(",").some(sym => syms.has(sym))) {
        token.classList.add("sw-brushed");
      }
    }
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

  // The hues around a family's of its lines (the root's first), and their
  // lightnesses (alternating, and then darker and lighter for more lines).
  static FAMILY_OFFSETS = [0, 30, -30, 60, -60, 15, -15, 45, -45, 75, -75, 90, -90];
  static LIGHTNESS = [42, 60, 32, 52];

  /**
   * Each line's color: a shade of its family's hue (by its root's semantic
   * type) of its own, in the order its lines are reached from the family's
   * roots (breadth first, so parents before what's derived from them).
   */
  allocateColors() {
    const lines = this.model.lines;
    const children = new Map(lines.map(line => [line.id, []]));
    for (const line of lines) {
      for (const parent of line.parents) {
        children.get(parent)?.push(line.id);
      }
    }
    const colors = new Map();
    for (const family of this.model.families) {
      const hue = this.hueOf(family.semantic ?? family.label);
      const roots = lines.filter(line => line.family == family.id && !line.parents.length);
      roots.sort((a, b) => (a.id == family.root ? -1 : 0) - (b.id == family.root ? -1 : 0));
      const order = [];
      const queue = roots.map(root => root.id);
      const seen = new Set(queue);
      while (queue.length) {
        const id = queue.shift();
        order.push(id);
        for (const kid of children.get(id)) {
          if (!seen.has(kid)) {
            seen.add(kid);
            queue.push(kid);
          }
        }
      }
      // (Lines of the family reached from none of its roots, last.)
      for (const line of lines) {
        if (line.family == family.id && !seen.has(line.id)) {
          order.push(line.id);
        }
      }
      const offsets = SubwayMap.FAMILY_OFFSETS;
      const lightness = SubwayMap.LIGHTNESS;
      order.forEach((id, k) => {
        const h = ((hue + offsets[k % offsets.length]) % 360 + 360) % 360;
        const light = lightness[(k + Math.floor(k / offsets.length)) % lightness.length];
        colors.set(id, `hsl(${h}, 68%, ${light}%)`);
      });
    }
    for (const line of lines) {
      if (!colors.has(line.id)) {
        colors.set(line.id, `hsl(${this.hueOf(line.semantic ?? line.label)}, 68%, 42%)`);
      }
    }
    return colors;
  }

  addStyles() {
    TrustedHTML.adoptStyleSheet().replaceSync(this.model.lines.map(line =>
      `.interactive-graph-edge-line-${line.id}:not(.clicktarget),
       .interactive-graph-edge-line-${line.id}-tainted:not(.clicktarget) {
         stroke: ${this.colors.get(line.id)}; stroke-width: 3px; stroke-opacity: 1;
       }`
    ).join("\n") + `
      [class*="interactive-graph-edge-line-"][class$="-tainted"]:not(.clicktarget) {
        stroke-dasharray: 6 3;
        filter: drop-shadow(0 0 1.5px rgb(210, 0, 0));
      }`);
    this.focusSheet = TrustedHTML.adoptStyleSheet();
  }

  /** Highlight a line, or lines (or stop, with null). */
  focus(ids) {
    ids = ids == null ? [] : [].concat(ids);
    this.focusSheet.replaceSync(!ids.length ? "" :
      `.interactive-graph-edge${ids.map(id =>
        `:not(.interactive-graph-edge-line-${id}):not(.interactive-graph-edge-line-${id}-tainted)`).join("")},
       .sw-track${ids.map(id => `:not([data-line="${id}"])`).join("")} { opacity: 0.15; }`);
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

  /**
   * The lines in a card, and where each is, by its rows' positions (`rows`,
   * flattened across its stations): from the first row it stops at (or
   * comes in at: the top, -1, or for a return, its call's row) to the last
   * (or the bottom, `rows.length`, where it goes on from the card).  A row's
   * stops are its events' (`Row::stops`), or for maps without them, its
   * station's, at the station's middle row.
   */
  cardTracks(card) {
    const rows = [];
    const firstStop = [];
    card.stations.forEach((station, si) => {
      const start = rows.length;
      if (!station.rows.some(row => row.stops)) {
        const middle = station.rows[Math.floor((station.rows.length - 1) / 2)];
        if (middle) {
          middle.stops = station.stops;
          middle.writes = station.writes;
        }
      }
      station.rows.forEach((row, ri) => rows.push({ si, ri, row }));
      const stopping = station.rows.findIndex(row => row.stops?.length);
      firstStop.push(start + (stopping >= 0 ? stopping : Math.floor((station.rows.length - 1) / 2)));
    });
    const spans = new Map();
    // (Its lines, whether or not they have tracks: the gutter's as wide
    // before the layout as after it, when the sides they go on to are known.)
    const present = new Set();
    const extend = (line, position) => {
      if (this.hiddenLines.has(line)) {
        return;
      }
      present.add(line);
      if (position == null) {
        return;
      }
      const span = spans.get(line) ?? { first: position, last: position };
      span.first = Math.min(span.first, position);
      span.last = Math.max(span.last, position);
      spans.set(line, span);
    };
    rows.forEach(({ row }, position) => (row.stops ?? []).forEach(line => extend(line, position)));
    // (A taint source, a parameter, comes in at the top.)
    for (const source of this.model.taint?.sources ?? []) {
      if (source.card == card.id) {
        extend(source.line, -1);
      }
    }
    // (Where a line stops in a station: its first row there, else the
    // station's first stop's.)
    const stopIn = (si, line) => {
      const at = rows.findIndex(({ si: s, row }) => s == si && row.stops?.includes(line));
      return at >= 0 ? at : firstStop[si];
    };
    // (Where its lines come in and go on: once the graph's laid out, at the
    // sides their edges are at (see `afterLayout`), else at the top, or a
    // return's call's row, and at the bottom.)
    const sides = this.sides?.get(card.id);
    for (const c of this.model.connectors) {
      for (const line of c.lines) {
        if (c.to == card.id) {
          extend(line, c.to_station != null ? stopIn(c.to_station, line) : this.sides ? null : -1);
        }
        if (c.from == card.id) {
          // (From a call's station's row, where its argument goes out, as
          // the callee's parameter's line (ex: `aValue`), which has no stops
          // in the caller.)
          if (c.from_station != null) {
            extend(line, stopIn(c.from_station, line));
          }
          extend(line, this.sides ? null : rows.length);
        }
      }
    }
    for (const [line, at] of sides ?? []) {
      if (at.has("top")) {
        extend(line, -1);
      }
      if (at.has("bottom")) {
        extend(line, rows.length);
      }
    }
    const order = [...present].sort((a, b) =>
      this.model.lines[a].family - this.model.lines[b].family || a - b);
    return { card, order, spans, rows, firstStop, stopIn };
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

  /**
   * The gutter at a row's position (or between two, at x.5, for gaps): its
   * tracks, from their first stops' rows' middles to their last's, tainted
   * as they arrive at a station (above its first stop's row) or leave it.
   */
  gutter(tracks, position) {
    const NS = "http://www.w3.org/2000/svg";
    const width = Math.max(1, tracks.order.length) * SubwayMap.TRACK;
    const svg = document.createElementNS(NS, "svg");
    svg.classList.add("sw-gutter");
    svg.dataset.order = tracks.order.join(",");
    svg.setAttribute("width", width);
    svg.setAttribute("viewBox", `0 0 ${width} 10`);
    svg.setAttribute("preserveAspectRatio", "none");
    const whole = Number.isInteger(position);
    const ceil = Math.ceil(position);
    const si = tracks.rows[Math.min(ceil, tracks.rows.length - 1)]?.si ?? 0;
    const split = tracks.firstStop[si];
    tracks.order.forEach((line, i) => {
      const span = tracks.spans.get(line);
      if (!span || position < span.first || position > span.last) {
        return;
      }
      let top = 0;
      let bottom = 10;
      if (position == span.first) {
        top = 5;
      }
      if (position == span.last && span.last < tracks.rows.length) {
        bottom = 5;
      }
      if (top >= bottom) {
        // (A line stopping at one row of a card, without coming or going,
        // is a stub there, so that its color shows.)
        top = 1;
        bottom = 9;
      }
      const parts = [];
      // (Arriving at its station above its first stop's row, and leaving
      // below; gaps between stations are arriving at the next.)
      const k = si;
      if (!whole) {
        parts.push([top, bottom, this.taintedBefore(tracks.card, line, position < split ? k : k + 1)]);
      } else if (position < split) {
        parts.push([top, bottom, this.taintedBefore(tracks.card, line, k)]);
      } else if (position > split) {
        parts.push([top, bottom, this.taintedBefore(tracks.card, line, k + 1)]);
      } else {
        parts.push([top, Math.max(top, 5), this.taintedBefore(tracks.card, line, k)]);
        parts.push([Math.max(top, 5), bottom, this.taintedBefore(tracks.card, line, k + 1)]);
      }
      const x = i * SubwayMap.TRACK + SubwayMap.TRACK / 2;
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
   * The tracks of the lines coming in at a card's top (see `afterLayout`)
   * through its head, over its border, to its first row's, arriving at its
   * first station.
   */
  headTracks(tracks) {
    const NS = "http://www.w3.org/2000/svg";
    const width = Math.max(1, tracks.order.length) * SubwayMap.TRACK;
    const svg = document.createElementNS(NS, "svg");
    svg.classList.add("sw-gutter", "sw-head-tracks");
    svg.dataset.order = tracks.order.join(",");
    svg.setAttribute("width", width);
    svg.setAttribute("viewBox", `0 0 ${width} 10`);
    svg.setAttribute("preserveAspectRatio", "none");
    tracks.order.forEach((line, i) => {
      if (tracks.spans.get(line)?.first != -1) {
        return;
      }
      const x = i * SubwayMap.TRACK + SubwayMap.TRACK / 2;
      const segment = document.createElementNS(NS, "line");
      segment.setAttribute("x1", x);
      segment.setAttribute("x2", x);
      segment.setAttribute("y1", 0);
      segment.setAttribute("y2", 10);
      segment.setAttribute("stroke", this.colors.get(line));
      segment.classList.add("sw-track");
      segment.classList.toggle("sw-tainted", this.taintedBefore(tracks.card, line, 0));
      segment.dataset.line = line;
      svg.append(segment);
    });
    return svg;
  }

  /**
   * A row's dots (the lines stopping at its events), over its gutter, and
   * its station's validations' rings (see `subway::Validation`), at the rows
   * where their lines stop.
   */
  dots(tracks, position) {
    const { si, row } = tracks.rows[position];
    const station = tracks.card.stations[si];
    const stops = row.stops ?? [];
    const writes = row.writes ?? [];
    const validations = (this.model.taint?.validations ?? []).filter(v =>
      v.card == tracks.card.id && v.station == si);
    const box = document.createElement("span");
    box.className = "sw-dots";
    tracks.order.forEach((line, i) => {
      const checks = validations.filter(v =>
        v.lines.includes(line) && tracks.stopIn(si, line) == position);
      if (!stops.includes(line) && !checks.length) {
        return;
      }
      const dot = document.createElement("span");
      dot.className = "sw-dot sw-track" + (writes.includes(line) ? " sw-dot-write" : "");
      dot.dataset.line = line;
      // (In its line's color, but validations' rings.)
      if (!checks.length) {
        dot.style.borderColor = this.colors.get(line);
        if (writes.includes(line)) {
          dot.style.backgroundColor = this.colors.get(line);
        }
      }
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
    block.swTracksKey = this.tracksKey(tracks);
    element.classList.add("sw-card");
    element.classList.toggle("sw-card-changed", card.changed);
    element.classList.toggle("sw-card-seed", card.seed ?? false);
    element.style.setProperty("--sw-gutter", `${Math.max(1, tracks.order.length) * SubwayMap.TRACK}px`);
    // (Its lines' edges end at their tracks, from its border's left edge:
    // its left border's width and the track's x in its rows' gutters, which
    // start its rows; and they go over its top and bottom borders, 1px, to
    // its tracks' ends; see RowLayout's `portInset`.)
    block.portInset = 1;
    block.portX = edge => {
      const line = Number(/^line-(\d+)/.exec(edge.kind)?.[1]);
      const i = tracks.order.indexOf(line);
      if (i < 0) {
        return element.offsetWidth / 2;
      }
      const row = element.querySelector(".sw-row");
      return element.clientLeft + (row?.offsetLeft ?? 0) + i * SubwayMap.TRACK + SubwayMap.TRACK / 2;
    };

    const head = document.createElement("div");
    head.className = "sw-head";
    if (tracks.order.some(line => tracks.spans.get(line)?.first == -1)) {
      head.append(this.headTracks(tracks));
    }
    const name = document.createElement("span");
    name.className = "sw-name";
    name.textContent = card.func;
    const where = document.createElement("a");
    where.className = "sw-where";
    const firstLine = card.stations[0]?.rows[0]?.line ?? 1;
    where.href = this.sourceURL(card.path, firstLine);
    where.textContent = `${card.path.split("/").pop()}:${firstLine}`;
    head.append(name, " ", where);
    // (Its calls of black boxes' functions, collapsed, which the trace didn't
    // step into: a chip naming them, whose menu shows their internals; see
    // follow-internals.js.)
    const blackBoxes = card.black_boxes ?? [];
    if (blackBoxes.length) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "sw-black-box";
      chip.textContent = `\u25A3 ${blackBoxes.map(bb => bb.name).join(", ")}`;
      chip.title = blackBoxes.map(bb =>
        `${bb.name}: ${bb.calls.join(", ")} (line${bb.lines.length == 1 ? "" : "s"} ${bb.lines.join(", ")}): ${bb.why}`)
        .join("\n");
      chip.addEventListener("click", event => {
        event.stopPropagation();
        this.showBlackBoxMenu(blackBoxes, event);
      });
      head.append(" ", chip);
    }
    const blackBoxLines = new Map();
    for (const bb of blackBoxes) {
      for (const line of bb.lines) {
        blackBoxLines.set(line, [...(blackBoxLines.get(line) ?? []), bb.name]);
      }
    }
    element.append(head);

    let position = 0;
    let shownLine = null;
    card.stations.forEach((station, si) => {
      // (Its rows, which hovering highlights, with its lines.)
      const box = document.createElement("div");
      box.className = "sw-station";
      box.dataset.key = `${card.id}:${si}`;
      box.dataset.stops = station.stops.filter(line => !this.hiddenLines.has(line)).join(",");
      if (si > 0) {
        const gap = document.createElement("div");
        gap.className = "sw-row sw-gap";
        gap.append(this.gutter(tracks, position - 0.5));
        const text = document.createElement("span");
        text.className = "sw-gap-text";
        text.textContent = "⋮";
        gap.append(text);
        element.append(gap);
      }
      element.append(box);
      station.rows.forEach(row => {
        if (row.skipped) {
          const skip = document.createElement("div");
          skip.className = "sw-row sw-skip";
          skip.append(this.gutter(tracks, position - 0.5));
          const text = document.createElement("span");
          text.className = "sw-gap-text";
          text.textContent = `⋮ ${row.skipped} line${row.skipped == 1 ? "" : "s"}`;
          skip.append(text);
          box.append(skip);
        }
        const div = document.createElement("div");
        div.className = "sw-row" + (row.changed ? " sw-changed" : "") +
          (row.traced ? " sw-traced" : "") +
          (blackBoxLines.has(row.line) && !row.node ? " sw-black-box-row" : "") +
          (row.composite ? " sw-composite-head" : "") +
          (row.node ? " sw-composite-arg" : "") +
          (row.argEnd ? " sw-composite-end" : "");
        if (blackBoxLines.has(row.line)) {
          div.title = `Calls into ${blackBoxLines.get(row.line).join(", ")}, collapsed`;
        }
        div.append(this.gutter(tracks, position), this.dots(tracks, position));
        const number = document.createElement("a");
        number.className = "sw-lno";
        number.href = this.sourceURL(card.path, row.line);
        // (Once for a line's rows, as a macro's arguments can share one.)
        number.textContent = row.line == shownLine ? "" : row.line;
        shownLine = row.line;
        const code = document.createElement("code");
        code.className = "sw-code";
        if (row.composite) {
          // (A macro's invocation's block's head: its name, and its
          // signature as its tooltip.)
          const name = document.createElement("span");
          name.className = "sw-composite-name";
          name.textContent = row.composite.name;
          name.title = row.composite.signature;
          code.append(name);
        } else if (row.node) {
          // (An argument's lines: the first labeled with what it is.)
          const label = document.createElement("span");
          label.className = "sw-composite-label";
          label.textContent = row.label;
          code.append(label, row.node.cloneNode(true));
        } else if (row.html) {
          TrustedHTML.set(code, row.html);
        } else {
          code.textContent = row.text;
        }
        div.append(number, code);
        box.append(div);
        position++;
      });
    });
  }

  rerender() {
    this.graph?.setInput(this.input());
  }

  /**
   * Once the graph's laid out in rows (whose edges end at their lines'
   * tracks, see `renderCard`'s `portX`), each card's tracks go on to the
   * sides its lines' edges are at (`sides`, by card and line): down to its
   * bottom for an edge to a card below, up to its top for one to a card
   * above (ex: a return to its caller, which is laid out above it), and for
   * an edge to a card in its row, which goes down and back up, its bottom.
   */
  afterLayout(graph) {
    if (!graph.rowLayout) {
      this.sides = null;
      return;
    }
    const sides = new Map();
    const add = (block, line, side) => {
      const id = block.data?.card?.id;
      if (id == null) {
        return;
      }
      const lines = sides.get(id) ?? new Map();
      sides.set(id, lines);
      lines.set(line, (lines.get(line) ?? new Set()).add(side));
    };
    for (const edge of graph.edges) {
      const match = /^line-(\d+)/.exec(edge.kind);
      if (!match || edge.loopback || !edge.rowSegments) {
        continue;
      }
      const line = Number(match[1]);
      const sameRow = edge.rowSegments.some(segment => segment.sameRow);
      add(edge.pred, line, sameRow || !edge.upward ? "bottom" : "top");
      add(edge.succ, line, sameRow || edge.upward ? "bottom" : "top");
    }
    this.sides = sides;
    // (An edge going up, ex: a return to its caller laid out above its
    // callee, or one to a card in its row, which goes down and back up: a
    // chevron where it comes into its card, as edges otherwise go down.)
    const NS = "http://www.w3.org/2000/svg";
    for (const edge of graph.edges) {
      const sameRow = edge.rowSegments?.some(segment => segment.sameRow);
      const points = edge.layoutPoints;
      if (!edge.node || !points?.length || !/^line-/.test(edge.kind) || !(edge.upward || sameRow)) {
        continue;
      }
      // (Its succ's end: the upper one, going up, or the last, back up.)
      const { x, y } = sameRow ? points[points.length - 1] : points[0];
      const head = document.createElementNS(NS, "path");
      head.setAttribute("d", `M ${x - 4} ${y + 7} L ${x} ${y + 1} L ${x + 4} ${y + 7}`);
      head.setAttribute("class", `interactive-graph-edge interactive-graph-edge-${edge.kind} sw-up`);
      edge.node.append(head);
    }
    // (Those whose tracks are others now, rendered again, as they were
    // rendered for the layout before it was known where their edges are:
    // their sizes are the same.)
    for (const block of graph.blocks) {
      if (block.node && block.data?.card &&
          this.tracksKey(this.cardTracks(block.data.card)) != block.swTracksKey) {
        block.node.replaceChildren();
        this.renderCard(block, block.node);
      }
    }
  }

  /** A card's tracks' spans, to tell whether they've changed. */
  tracksKey(tracks) {
    return JSON.stringify(tracks.order.map(line => [line, tracks.spans.get(line)]));
  }

  /**
   * A card's invocations of macros with signatures (see the tools'
   * `subway::Composite`, from dataflow-mapping.toml) as blocks of their
   * arguments: a row naming the macro, then each argument's lines, the first
   * labeled with what it is (ex: "tries"), their code the rows' split at its
   * columns; in place of the invocation's rows in their station, so that the
   * tracks' positions are the shown rows'.  An argument with a token of the
   * values stopping there is traced, and the lines stopping at the
   * invocation stop at its first traced argument (or else its name).
   */
  expandComposites(card) {
    for (const composite of card.composites ?? []) {
      const station = card.stations.find(s =>
        s.rows.some(row => row.line == composite.line) &&
        s.rows.some(row => row.line == composite.end_line));
      if (!station) {
        continue;
      }
      const first = station.rows.findIndex(row => row.line == composite.line);
      const last = station.rows.findIndex(row => row.line == composite.end_line);
      if (last < first) {
        continue;
      }
      const source = station.rows.slice(first, last + 1);
      const byLine = new Map(source.map(row => [row.line, row]));
      const head = {
        line: composite.line,
        composite,
        changed: source.some(row => row.changed),
        traced: source.some(row => row.traced),
        skipped: source[0].skipped,
      };
      const rows = [head];
      let stopsAt = head;
      // (And the lines connected at the station, as an argument passed to a
      // call isn't a stop of its own.)
      const si = card.stations.indexOf(station);
      const connected = this.model.connectors
        .filter(c => (c.from == card.id && c.from_station == si) || (c.to == card.id && c.to_station == si))
        .flatMap(c => c.lines);
      const stoppingNames = new Set([...source.flatMap(row => row.stops ?? []), ...connected]
        .flatMap(line => this.model.lines[line]?.names ?? []));
      composite.args.forEach((arg, ai) => {
        const argRows = [];
        for (let line = arg.line; line <= arg.end_line; line++) {
          const row = byLine.get(line);
          if (!row) {
            continue;
          }
          // (Its later lines without their indentation up to its first's
          // column, as it's no longer after the macro's name.)
          const indent = (row.text ?? "").match(/^\s*/)[0].length;
          argRows.push({
            line,
            label: line == arg.line ? arg.label || "\u2026" : "",
            node: this.rowSlice(row, line == arg.line ? arg.col : Math.min(arg.col, indent),
                                line == arg.end_line ? arg.end_col : Infinity),
            changed: row.changed,
            argEnd: ai == composite.args.length - 1 && line == arg.end_line,
          });
        }
        // (By its tokens' symbols, or their names, of the values stopping at
        // the invocation, as a token's symbol can be another than its
        // value's line has.)
        const traced = argRows.some(row => [...row.node.querySelectorAll("[data-symbols]")]
          .some(token => token.dataset.symbols.split(",").some(sym => this.valueSyms.has(sym)) ||
                stoppingNames.has(token.textContent)));
        for (const row of argRows) {
          row.traced = traced;
        }
        if (traced && stopsAt == head && argRows.length) {
          stopsAt = argRows[0];
        }
        rows.push(...argRows);
      });
      stopsAt.stops = [...new Set(source.flatMap(row => row.stops ?? []))];
      stopsAt.writes = [...new Set(source.flatMap(row => row.writes ?? []))];
      station.rows.splice(first, last - first + 1, ...rows);
    }
  }

  /** A row's code (its HTML's nodes) from one UTF-16 column to another. */
  rowSlice(row, start, end) {
    const code = document.createElement("code");
    if (row.html) {
      TrustedHTML.set(code, row.html);
    } else {
      code.textContent = row.text;
    }
    const range = document.createRange();
    range.selectNodeContents(code);
    const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
    let offset = 0;
    let started = false;
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const length = node.data.length;
      if (!started && start <= offset + length) {
        range.setStart(node, Math.max(0, start - offset));
        started = true;
      }
      if (end <= offset + length) {
        range.setEnd(node, Math.max(0, end - offset));
        break;
      }
      offset += length;
    }
    const slice = document.createElement("span");
    slice.append(range.cloneContents());
    return slice;
  }

  /**
   * The menu of a card's collapsed black boxes: showing their internals,
   * each's (in a submenu, if there are several) or all of them.
   */
  showBlackBoxMenu(blackBoxes, event) {
    const show = bb => new MenuItem({
      html: ContextMenu.fmt("Show internals of <strong>_</strong>", bb.name),
      href: FollowInternals.url([bb.group]),
      icon: "search",
      section: "black-boxes",
    });
    const items = blackBoxes.length == 1 ? [show(blackBoxes[0])] : [new MenuItemWithSubMenu({
      html: "Show internals",
      section: "black-boxes",
      menu: ContextMenu,
      searches: [],
      items: [
        ...blackBoxes.map(show),
        new MenuItem({
          html: "Of all of them",
          href: FollowInternals.url(blackBoxes.map(bb => bb.group)),
          icon: "search",
          section: "black-boxes-all",
        }),
      ],
    })];
    ContextMenu.showItems(items, event);
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
    const trace = this.model.trace;
    params.set("q", `${params.get("q") ?? ""} ${trace ? "follow-format" : "slice-format"}:svg`);
    dot.href = `${document.location.pathname}?${params}`;
    dot.textContent = "Dataflow graph";
    dot.title = trace
      ? "The trace's steps, as a graph"
      : "The clusters' changes and the dataflow linking them, as a graph";
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
      summary.textContent = trace?.title ??
        (`Cluster ${cluster.n}` + (cluster.commits ? ` (${cluster.commits})` : ""));
      const list = document.createElement("ul");
      list.className = "sw-legend-families";
      list.append(...families.map(familyItem));
      section.append(summary, list);
      legend.append(section);
    }

    // (The black boxes the trace didn't step into, with links showing their
    // internals.)
    const blackBoxes = new Map();
    for (const card of this.model.cards) {
      for (const bb of card.black_boxes ?? []) {
        const entry = blackBoxes.get(bb.group) ?? { bb, cards: 0 };
        entry.cards++;
        blackBoxes.set(bb.group, entry);
      }
    }
    if (blackBoxes.size) {
      const section = document.createElement("details");
      section.open = true;
      section.className = "sw-legend-black-boxes";
      const summary = document.createElement("summary");
      summary.textContent = "Collapsed";
      const list = document.createElement("ul");
      const link = (text, groups) => {
        const a = document.createElement("a");
        a.href = FollowInternals.url(groups);
        a.textContent = text;
        return a;
      };
      for (const { bb, cards } of blackBoxes.values()) {
        const item = document.createElement("li");
        item.title = bb.why;
        item.append(`\u25A3 ${bb.name} (${cards} function${cards == 1 ? "" : "s"}): `,
                    link("show internals", [bb.group]));
        list.append(item);
      }
      if (blackBoxes.size > 1) {
        const item = document.createElement("li");
        item.append(link("Show all of their internals", [...blackBoxes.keys()]));
        list.append(item);
      }
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
      // (A parameter's fields alike are an item, ex: a struct's principals.)
      const groups = new Map();
      for (const source of sources) {
        const key = [source.card, source.line, source.status, source.stops, source.tainted_stops].join(":");
        if (!groups.has(key)) {
          groups.set(key, { source, fields: [] });
        }
        if (source.field) {
          groups.get(key).fields.push(source.field);
        }
      }
      for (const { source, fields } of groups.values()) {
        const item = document.createElement("li");
        const status = document.createElement("span");
        status.className = `sw-status sw-status-${source.status}`;
        status.textContent = source.status;
        status.title = `${source.tainted_stops} of its ${source.stops} stops tainted`;
        const label = this.model.lines[source.line].label;
        // (Its parameter's, rather than its first field's: "receives aArgs".)
        const why = fields.length > 1 ?
          source.why.replace(/ receives ([^ .]+)\.\S+ \([^)]*\)/, " receives $1") : source.why;
        item.append(status, ` \u2623 ${label}` +
          (fields.length > 1 ? ` (${fields.length} fields: ${fields.join(", ")})` : "") + `: ${why}`);
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
