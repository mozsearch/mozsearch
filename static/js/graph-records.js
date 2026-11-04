"use strict";

// Diagrams' records (lock-held.js's): a layered call graph (graph.js's
// `RowLayout`) whose rows can be very wide where a function calls many
// methods of a class (ex: a constructor's getters and setters), so the nodes
// of a row which are the same scope's (a class's, by `scopeOf`) are a
// record's cells instead, a table of them, as the old Graphviz diagrams' HTML
// labels' records were: its scope over its first column, and its cells read
// down its columns, each pair of them beside a raceway (or a single column
// beside its own), along which the edges to and from its cells run, as
// cables in a cable tray do, each its own lane, and peel off to their cells.
// The edges between the same two places (a block, or a record's raceway) are
// one cable, a bus, drawn thicker than an edge, and the layout lays out the
// cables, not each edge: so a function calling 30 of a class's methods is one
// edge down to a record of them, not 30 edges fanning out over a row of 30
// blocks.  Hovering over a block or a cell highlights its edges' stubs and
// their other ends' cells.

// How many of a row's nodes of the same scope are a record, at least; how
// many cells a record of a single column has at most, and how many rows a
// record has at most; and its raceways' widths (at least; their lanes', and
// their margins), and the gap between its pairs of columns.
const RECORD_MIN_CELLS = 3;
const RECORD_SINGLE_COLUMN = 5;
const RECORD_MAX_ROWS = 10;
const RACEWAY_MIN = 12;
const RACEWAY_LANE_GAP = 5;
const RACEWAY_MARGIN = 5;
const RECORD_PANEL_GAP = 10;

/**
 * Lays out `nodes` and `edges` (each `{ from, to }`, the nodes' indices) in
 * `canvas`, with records of a row's nodes of the same scope (see the file's
 * comment): `makeNode(node, index)` makes a node's element (a block's, or a
 * record's cell's), `scopeOf(node, index)` says its scope (or null, if it
 * isn't a record's), and `edgeTitle(edge)` an edge's tooltip.  Returns the
 * layout's `size`, the nodes' `elements`, each edge's `paths` (its stubs, and
 * its cable's, if it's alone in it), and the `cables` (each one's `edges` and
 * `paths`).
 */
function drawRecordGraph(canvas, nodes, edges, { makeNode, scopeOf, edgeTitle, margin = 6 }) {
  const NS = "http://www.w3.org/2000/svg";
  const elements = nodes.map((node, i) => makeNode(node, i));

  // The rows the nodes are in (as the layout's), for their records.
  const probeItems = nodes.map(() => ({ width: 1, height: 1 }));
  const probe = new RowLayout(probeItems,
    edges.map(edge => ({ pred: probeItems[edge.from], succ: probeItems[edge.to] })));
  probe.assignRows();
  const depths = probeItems.map(item => probe.nodeOf.get(item).depth);

  // The records: a row's nodes of a scope, if they're enough, and don't call
  // each other.
  const groups = new Map();
  nodes.forEach((node, i) => {
    const scope = scopeOf(node, i);
    if (!scope) {
      return;
    }
    const key = `${depths[i]}\n${scope}`;
    if (!groups.has(key)) {
      groups.set(key, { scope, cells: [] });
    }
    groups.get(key).cells.push(i);
  });
  const records = [];
  const recordOf = new Array(nodes.length).fill(null);
  for (const group of groups.values()) {
    const members = new Set(group.cells);
    if (group.cells.length < RECORD_MIN_CELLS ||
        edges.some(edge => members.has(edge.from) && members.has(edge.to))) {
      continue;
    }
    records.push(group);
    group.cells.forEach(i => { recordOf[i] = group; });
  }

  // Each record's cells, by their callers (so a cable's are together), down
  // its columns, and each one's column, row, raceway, and side facing it.
  const cellAt = new Array(nodes.length).fill(null);
  const firstCaller = i => Math.min(nodes.length, ...edges.filter(edge => edge.to == i).map(edge => edge.from));
  for (const record of records) {
    record.cells.sort((a, b) => firstCaller(a) - firstCaller(b) || a - b);
    const n = record.cells.length;
    record.columns = n <= RECORD_SINGLE_COLUMN ? 1 : 2 * Math.ceil(n / (2 * RECORD_MAX_ROWS));
    record.rows = Math.ceil(n / record.columns);
    record.raceways = Math.ceil(record.columns / 2);
    record.lanes = Array.from({ length: record.raceways }, () => []);
    record.cells.forEach((i, k) => {
      const column = Math.floor(k / record.rows);
      cellAt[i] = {
        record,
        column,
        row: k % record.rows,
        raceway: record.columns == 1 ? 0 : column >> 1,
        // (The raceway's on its left, but a pair's left column's.)
        side: record.columns > 1 && column % 2 == 0 ? "right" : "left",
      };
    });
  }

  // The cables: the edges between the same two places, a block or a
  // record's raceway.
  const place = i => (cellAt[i]
    ? { record: cellAt[i].record, raceway: cellAt[i].raceway, key: `${records.indexOf(cellAt[i].record)}:${cellAt[i].raceway}` }
    : { node: i, key: `n${i}` });
  const cableOf = new Map();
  const cables = [];
  edges.forEach((edge, e) => {
    const from = place(edge.from);
    const to = place(edge.to);
    if ((from.record && from.record === to.record) || from.node === edge.to) {
      return;
    }
    const key = `${from.key}>${to.key}`;
    if (!cableOf.has(key)) {
      const cable = { from, to, edges: [], paths: [] };
      cableOf.set(key, cable);
      cables.push(cable);
    }
    cableOf.get(key).edges.push(e);
  });
  // (Each raceway's lanes: the cables into it, then those out of it, by
  // their other ends.)
  const otherEnd = (cable, end) => Math.min(...cable.edges.map(e => edges[e][end]));
  for (const cable of [...cables].sort((a, b) => otherEnd(a, "from") - otherEnd(b, "from"))) {
    if (cable.to.record) {
      cable.toLane = cable.to.record.lanes[cable.to.raceway].push(cable) - 1;
    }
  }
  for (const cable of [...cables].sort((a, b) => otherEnd(a, "to") - otherEnd(b, "to"))) {
    if (cable.from.record) {
      cable.fromLane = cable.from.record.lanes[cable.from.raceway].push(cable) - 1;
    }
  }

  // The records' elements: their scopes over their first columns, their
  // cells in their columns' tracks, beside their raceways' tracks (marked,
  // to measure them).
  for (const record of records) {
    const element = document.createElement("div");
    element.className = "graph-record";
    const width = lanes => Math.max(RACEWAY_MIN, 2 * RACEWAY_MARGIN + (lanes - 1) * RACEWAY_LANE_GAP);
    const tracks = [];
    const columnTracks = [];
    const racewayTracks = [];
    if (record.columns == 1) {
      racewayTracks.push(tracks.push(`${width(record.lanes[0].length)}px`));
      columnTracks.push(tracks.push("auto"));
    } else {
      for (let p = 0; p < record.raceways; p++) {
        if (p) {
          tracks.push(`${RECORD_PANEL_GAP}px`);
        }
        columnTracks.push(tracks.push("auto"));
        racewayTracks.push(tracks.push(`${width(record.lanes[p].length)}px`));
        columnTracks.push(tracks.push("auto"));
      }
    }
    element.style.gridTemplateColumns = tracks.join(" ");
    // (A class's namespaces over its name.)
    const scope = document.createElement("div");
    scope.className = "graph-record-scope";
    const m = record.scope.match(/^(.*::)?([^:]+)::$/);
    const outer = document.createElement("div");
    outer.textContent = m ? m[1] || "" : "";
    const name = document.createElement("div");
    name.className = "graph-record-scope-name";
    name.textContent = m ? m[2] : record.scope;
    scope.append(outer, name);
    scope.title = record.scope.replace(/::$/, "");
    scope.style.gridColumn = columnTracks[0];
    scope.style.gridRow = 1;
    element.append(scope);
    record.racewayMarks = racewayTracks.map(track => {
      const mark = document.createElement("div");
      mark.className = "graph-record-raceway";
      mark.style.gridColumn = track;
      mark.style.gridRow = `1 / ${record.rows + 2}`;
      element.append(mark);
      return mark;
    });
    for (const i of record.cells) {
      const cell = elements[i];
      cell.classList.add("graph-record-cell");
      cell.style.gridColumn = columnTracks[cellAt[i].column];
      cell.style.gridRow = 2 + cellAt[i].row;
      element.append(cell);
    }
    record.element = element;
    canvas.append(element);
  }

  // The layout's items: the blocks, and the records, whose ports are their
  // cables' lanes.
  const items = [];
  nodes.forEach((node, i) => {
    if (!cellAt[i]) {
      canvas.append(elements[i]);
    }
  });
  const blockItems = new Map();
  nodes.forEach((node, i) => {
    if (!cellAt[i]) {
      const item = { element: elements[i], width: elements[i].offsetWidth, height: elements[i].offsetHeight };
      blockItems.set(i, item);
      items.push(item);
    }
  });
  for (const record of records) {
    // (Offsets are from its padding's edge, inside its border.)
    record.inset = { x: record.element.clientLeft, y: record.element.clientTop };
    const raceways = record.racewayMarks.map(mark => ({ left: record.inset.x + mark.offsetLeft, width: mark.offsetWidth }));
    record.laneX = (raceway, lane) => {
      const { left, width } = raceways[raceway];
      const count = record.lanes[raceway].length;
      return left + (width - (count - 1) * RACEWAY_LANE_GAP) / 2 + lane * RACEWAY_LANE_GAP;
    };
    record.item = {
      element: record.element,
      width: record.element.offsetWidth,
      height: record.element.offsetHeight,
      portX: edge => (edge.cable.from.record === record
        ? record.laneX(edge.cable.from.raceway, edge.cable.fromLane)
        : record.laneX(edge.cable.to.raceway, edge.cable.toLane)),
    };
    items.push(record.item);
  }
  const placeItem = place => (place.record ? place.record.item : blockItems.get(place.node));
  const layoutEdges = cables.map(cable => ({ pred: placeItem(cable.from), succ: placeItem(cable.to), cable }));
  const size = new RowLayout(items, layoutEdges, { margin }).run();
  canvas.style.width = `${size.x}px`;
  canvas.style.height = `${size.y}px`;
  for (const item of items) {
    item.element.style.left = `${item.layoutLeft}px`;
    item.element.style.top = `${item.layoutTop}px`;
  }

  // The cables, along their raceways' lanes to their cells, and their
  // stubs, peeling off to them.
  const svg = document.createElementNS(NS, "svg");
  svg.classList.add("crash-graph-edges");
  svg.setAttribute("width", size.x);
  svg.setAttribute("height", size.y);
  const edgePaths = edges.map(() => []);
  const head = (x, y, dx, dy) => {
    const path = document.createElementNS(NS, "path");
    path.classList.add("crash-graph-edge-head");
    // (Pointing along (dx, dy), its tip at (x, y).)
    const [bx, by] = [x - 7 * dx, y - 7 * dy];
    path.setAttribute("d", `M ${x} ${y} L ${bx - 3 * dy} ${by - 3 * dx} L ${bx + 3 * dy} ${by + 3 * dx} Z`);
    return path;
  };
  // (A cell's center's height, and its side's x facing its raceway.)
  const cellY = i => {
    const record = cellAt[i].record;
    return record.item.layoutTop + record.inset.y + elements[i].offsetTop + elements[i].offsetHeight / 2;
  };
  const cellX = i => {
    const record = cellAt[i].record;
    const cell = elements[i];
    return record.item.layoutLeft + record.inset.x + cell.offsetLeft + (cellAt[i].side == "right" ? cell.offsetWidth : 0);
  };
  // (A cell's stubs in and out, apart, if it has both.)
  const stubSides = nodes.map(() => new Set());
  for (const cable of cables) {
    for (const e of cable.edges) {
      if (cable.from.record) {
        stubSides[edges[e].from].add("out");
      }
      if (cable.to.record) {
        stubSides[edges[e].to].add("in");
      }
    }
  }
  const stubY = (i, end) => cellY(i) + (stubSides[i].size > 1 ? (end == "to" ? -2 : 2) : 0);
  for (const edge of layoutEdges) {
    const cable = edge.cable;
    const points = (edge.layoutPoints || []).map(point => ({ ...point }));
    if (points.length < 2) {
      continue;
    }
    // (The pred's end is the first point, but an upward edge's.)
    const [predAt, succAt] = edge.upward ? [points.length - 1, 0] : [0, points.length - 1];
    // A record's end runs along its lane to the furthest of its cells.
    const lane = (end, at, cells) => {
      const record = cable[end].record;
      const port = points[at];
      const ys = cells.map(cellY);
      const top = port.y <= record.item.layoutTop + 1;
      return { x: port.x, y: top ? Math.max(...ys) : Math.min(...ys) };
    };
    const fromCells = cable.edges.map(e => edges[e].from);
    const toCells = cable.edges.map(e => edges[e].to);
    const ends = [];
    if (cable.from.record) {
      ends.push([predAt, lane("from", predAt, fromCells)]);
    }
    if (cable.to.record) {
      ends.push([succAt, lane("to", succAt, toCells)]);
    }
    for (const [at, point] of ends.sort((a, b) => b[0] - a[0])) {
      points.splice(at == 0 ? 0 : points.length, 0, point);
    }
    const trunk = document.createElementNS(NS, "path");
    trunk.classList.add("crash-graph-edge", "graph-cable");
    if (cable.edges.length > 1) {
      trunk.classList.add("graph-bus");
    }
    trunk.setAttribute("d", roundedPath(points));
    const title = document.createElementNS(NS, "title");
    title.textContent = cable.edges.length > 1
      ? `${cable.edges.length} calls`
      : edgeTitle(edges[cable.edges[0]]);
    trunk.append(title);
    svg.append(trunk);
    cable.paths.push(trunk);
    // (Its head at its callee: a block's, the line's end; a cell's, its
    // stub's.)
    if (!cable.to.record) {
      const end = edge.upward ? points[0] : points[points.length - 1];
      const before = edge.upward ? points[1] : points[points.length - 2];
      const arrow = head(end.x, end.y, 0, before.y > end.y ? -1 : 1);
      svg.append(arrow);
      cable.paths.push(arrow);
    }
    if (cable.edges.length == 1) {
      edgePaths[cable.edges[0]].push(...cable.paths);
    }
    // Its stubs, from its lanes to its cells.
    const laneXOf = end => (end == "from"
      ? points[edge.upward ? points.length - 1 : 0].x
      : points[edge.upward ? 0 : points.length - 1].x);
    for (const e of cable.edges) {
      for (const end of ["from", "to"]) {
        if (!cable[end].record) {
          continue;
        }
        const i = edges[e][end];
        const [x0, x1, y] = [laneXOf(end), cellX(i), stubY(i, end)];
        const stub = document.createElementNS(NS, "path");
        stub.classList.add("crash-graph-edge", "graph-stub");
        stub.setAttribute("d", `M ${x0} ${y} L ${x1} ${y}`);
        const stubTitle = document.createElementNS(NS, "title");
        stubTitle.textContent = edgeTitle(edges[e]);
        stub.append(stubTitle);
        svg.append(stub);
        edgePaths[e].push(stub);
        if (end == "to") {
          const arrow = head(x1, y, Math.sign(x1 - x0) || 1, 0);
          svg.append(arrow);
          edgePaths[e].push(arrow);
        }
      }
    }
  }
  canvas.prepend(svg);

  // Hovering over a node highlights its edges and their other ends.
  nodes.forEach((node, i) => {
    const touching = edges.map((edge, e) => e).filter(e => edges[e].from == i || edges[e].to == i);
    const hot = on => {
      for (const e of touching) {
        const edge = edges[e];
        for (const path of edgePaths[e]) {
          path.classList.toggle("graph-edge-hot", on);
        }
        elements[edge.from == i ? edge.to : edge.from].classList.toggle("graph-node-hot", on);
      }
      for (const cable of cables) {
        if (cable.edges.some(e => touching.includes(e))) {
          for (const path of cable.paths) {
            path.classList.toggle("graph-edge-hot", on);
          }
        }
      }
    };
    elements[i].addEventListener("mouseenter", () => hot(true));
    elements[i].addEventListener("mouseleave", () => hot(false));
  });

  return { size, elements, edges: edgePaths.map(paths => ({ paths })), cables };
}
