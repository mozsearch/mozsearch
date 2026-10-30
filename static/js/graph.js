const SVG_NS = "http://www.w3.org/2000/svg";
const ZOOM_SENSITIVITY = 1.5;
const WHEEL_DELTA_SCALE = 0.01;
const MAX_ZOOM = 2;
const MIN_ZOOM = 0.01;
const TRANSLATION_CLAMP_AMOUNT = 0;
const CROSS_MARGIN = 20;
const PORT_MARGIN = 10;
const CROSS_GAP = 8;
// The gap between the blocks with no edges, beside the grid (see
// `placeIsolatedBlocks`), and between them and it.
const ISOLATED_GAP = 24;

function clamp(x, min, max) {
  return Math.max(min, Math.min(max, x));
}

// How far apart (in px) an edge's ends between adjacent rows may be for it
// to be drawn straight instead (see `straightenPoints`).
const STRAIGHTEN_PX = 40;

/**
 * Draw an edge's corners' points straight if they make a small jog between
 * adjacent rows: an edge from `pred` down to `succ` whose route is down, a
 * short way across and down again (four points) is moved to one end's x, if
 * that's within the other end's block.  (As iongraph's "comb" passes
 * straighten nearly straight edges.)
 */
function straightenPoints(points, pred, succ) {
  if (points.length !== 4) {
    return points;
  }
  const [start, turn, , end] = points;
  // (Only down from one row to another, not across a row.)
  if (!(start.y < turn.y && turn.y < end.y)) {
    return points;
  }
  const dx = Math.abs(start.x - end.x);
  if (dx === 0 || dx > STRAIGHTEN_PX) {
    return points;
  }
  const within = (block, x) => block && x >= block.left + 6 && x <= block.left + block.width - 6;
  let x = null;
  if (within(succ, start.x)) {
    x = start.x;
  } else if (within(pred, end.x)) {
    x = end.x;
  }
  if (x === null) {
    return points;
  }
  return points.map(point => ({ x, y: point.y }));
}

/**
 * The SVG path of an edge through its corners' points, with rounded
 * corners (as graph.js's `createEdgeNodes` draws them).
 */
function roundedPath(points) {
  const R = 64;
  const T = 0.6;
  const start = points[0];
  let path = `M ${start.x} ${start.y} `;
  for (let i = 1; i < points.length - 1; i++) {
    const prev = points[i - 1];
    const curr = points[i];
    const next = points[i + 1];
    let c0x = curr.x, c0y = curr.y;
    let c3x = curr.x, c3y = curr.y;
    if (curr.x < prev.x) {
      c0x = Math.min(curr.x + R, (curr.x + prev.x) / 2);
    } else if (curr.x > prev.x) {
      c0x = Math.max(curr.x - R, (curr.x + prev.x) / 2);
    } else {
      c0y = Math.max(curr.y - R, (curr.y + prev.y) / 2);
    }
    if (next.x < curr.x) {
      c3x = Math.max(curr.x - R, (curr.x + next.x) / 2);
    } else if (next.x > curr.x) {
      c3x = Math.min(curr.x + R, (curr.x + next.x) / 2);
    } else {
      c3y = Math.min(curr.y + R, (curr.y + next.y) / 2);
    }
    const c1x = curr.x * T + c0x * (1 - T);
    const c1y = curr.y * T + c0y * (1 - T);
    const c2x = curr.x * T + c3x * (1 - T);
    const c2y = curr.y * T + c3y * (1 - T);
    path += `L ${c0x} ${c0y} `;
    path += `C ${c1x} ${c1y} ${c2x} ${c2y} ${c3x} ${c3y} `;
  }
  const end = points[points.length - 1];
  path += `L ${end.x} ${end.y} `;
  return path;
}

// Grid layout for the graph.
//
// This layout algorithm was heavily inspired by the iongraph layout algorithm.
// Because iongraph was specifically designed for control-flow layouts of
// (intra-procedular) basic blocks from a single function and to leverage
// constraints that allowed, it was not directly suitable to our
// inter-procedural control-flow diagrams. Specific reuses of code related to
// event handling are specifically called out within the source.
//
// Each block has 4 ports, and each port has multiple slots.
// An edge takes one slot from them, for each end.
//
// +--------+--------+------+--------+--------+------+
// | slot 1 | slot 2 | ...  | slot 1 | slot 2 | ...  |
// +--------+--------+------+--------+--------+------+
// | incoming port for down | outgoing port for up   |
// +------------------------+------------------------+
// | block content                                   |
// |                                                 |
// |                                                 |
// +------------------------+------------------------+
// | outgoing port for down | incoming port for up   |
// +--------+--------+------+--------+--------+------+
// | slot 1 | slot 2 | ...  | slot 1 | slot 2 | ...  |
// +--------+--------+------+--------+--------+------+
//
// Each block has adjacent horizontal gap and the vertical gap,
// and the crossroads between them.
//
// Horizontal gaps and vertical gaps have multiple slots.
// An edge takes at most one slot from one gap.
//
// +-------------------------+--------------------+
// | block (1,1)             | vertical (1,1)     |
// |                         +------+------+------+
// |                         | slot | slot | ...  |
// |                         | 1    | 2    |      |
// +-------------------------+------+------+------+
// | horizontal (1,1)        | cross              |
// +-------------------------+                    |
// | slot 1                  |                    |
// +-------------------------+                    |
// | slot 2                  |                    |
// +-------------------------+                    |
// | ...                     |                    |
// +-------------------------+------+------+------+
//
// The edge between the block at (1, 1) to the block at (2, 2)
// takes the following:
//
//   block (1,1) out-down slot 1
//   -> horz (1,1) slot 1
//   -> horz (1,2) slot 1
//   -> block (2,2) in-down slot 1
//
// +--------------+.......................
// | block (1,1)  |       .              .
// |              |       .              .
// +--------------+.......................
// . |            .       .              .
// . +----------------------+            .
// .              .       . |            .
// .              .       . v            .
// .......................+--------------+
// .              .       | block (2,2)  |
// .              .       |              |
// .              .       +--------------+
class GridLayout {
  constructor(maxBreadth, maxDepth) {
    // For simplicity, map[0][0] is for (-1,-1), so that all blocks has
    // gaps in the both side.
    this.map = [];
    this.maxBreadth = maxBreadth;
    this.maxDepth = maxDepth;

    for (let y = 0; y <= this.maxDepth + 1; y++) {
      const row = [];
      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        row.push({
          block: null,
          horz: [],
          vert: [],

          ports: {
            up: {
              in: [],
              out: [],
            },
            down: {
              in: [],
              out: [],
            },
          },

          // The size of this block.
          // This is always larger or equals to the size of the
          // actual block.width and block.height.
          blockWidth: 0,
          blockHeight: 0,

          crossWidth: 60,
          crossHeight: 60,

          blockLeft: 0,
          blockTop: 0,
        });
      }
      this.map.push(row);
    }
  }

  addBlock(breadth, depth, block) {
    const info = this.getInfoAt(breadth, depth);
    info.block = block;
  }

  // Sort the edges for each port, to make them shown better.
  sortPorts() {
    // Compare two edges on the same port, based on the following:
    //   * The angle of the node at the another side of the edge
    //   * If the angle is same, the distance of the edge
    function sorter(breadth, depth, dir, a, b) {
      let adx, ady, bdx, bdy;
      if (a.succ.breadth === breadth && a.succ.depth === depth) {
        adx = a.pred.breadth - breadth;
        ady = a.pred.depth - depth;
      } else {
        adx = a.succ.breadth - breadth;
        ady = a.succ.depth - depth;
      }

      if (b.succ.breadth === breadth && b.succ.depth === depth) {
        bdx = b.pred.breadth - breadth;
        bdy = b.pred.depth - depth;
      } else {
        bdx = b.succ.breadth - breadth;
        bdy = b.succ.depth - depth;
      }

      const ap = Math.atan2(ady * dir, adx);
      const bp = Math.atan2(bdy * dir, bdx);

      if (ap === bp) {
        return a.length > b.length;
      }

      return ap < bp;
    }

    for (let y = 0; y <= this.maxDepth + 1; y++) {
      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        const upSorter = sorter.bind(null, x - 1, y - 1, -1);
        const downSorter = sorter.bind(null, x - 1, y - 1, 1);
        this.map[y][x].ports.up.in.sort(upSorter);
        this.map[y][x].ports.up.out.sort(upSorter);
        this.map[y][x].ports.down.in.sort(downSorter);
        this.map[y][x].ports.down.out.sort(downSorter);
      }
    }
  }

  // Calculate the position of each block.
  //
  // The size of each block is set to the following:
  //   * the maximum width of the blocks in the same breadth
  //   * the maximum height of the blocks in the same depth
  //
  // The same applies to the crossroads.
  calculatePosition() {
    for (let y = 0; y <= this.maxDepth + 1; y++) {
      let crossHeight = 0;
      let blockHeight = 0;
      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        const info = this.map[y][x];
        crossHeight = Math.max(
          crossHeight,
          CROSS_MARGIN * 2 + (info.horz.length + 1) * CROSS_GAP);
        if (!info.block) {
          continue;
        }
        blockHeight = Math.max(blockHeight, info.block.height);
      }

      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        const info = this.map[y][x];
        info.crossHeight = crossHeight;
        info.blockHeight = blockHeight;
      }
    }

    for (let x = 0; x <= this.maxBreadth + 1; x++) {
      let crossWidth = 0;
      let blockWidth = 0;
      for (let y = 0; y <= this.maxDepth + 1; y++) {
        const info = this.map[y][x];
        crossWidth = Math.max(
          crossWidth,
          CROSS_MARGIN * 2 + (info.vert.length + 1) * CROSS_GAP);
        if (!info.block) {
          continue;
        }
        blockWidth = Math.max(blockWidth, info.block.width);
      }

      for (let y = 0; y <= this.maxDepth + 1; y++) {
        const info = this.map[y][x];
        info.crossWidth = crossWidth;
        info.blockWidth = blockWidth;
      }
    }

    for (let y = 0; y <= this.maxDepth + 1; y++) {
      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        if (x > 0) {
          this.map[y][x].blockLeft
            = this.map[y][x - 1].blockLeft
            + this.map[y][x - 1].blockWidth
            + this.map[y][x - 1].crossWidth;
        }
        if (y > 0) {
          this.map[y][x].blockTop
            = this.map[y - 1][x].blockTop
            + this.map[y - 1][x].blockHeight
            + this.map[y - 1][x].crossHeight;
        }
      }
    }

    for (let y = 0; y <= this.maxDepth + 1; y++) {
      let maxHorzLength = 0;
      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        maxHorzLength = Math.max(maxHorzLength, this.map[y][x].horz.length);
      }

      for (let x = 0; x <= this.maxBreadth + 1; x++) {
        this.map[y][x].horz.length = maxHorzLength;
      }
    }

    for (let x = 0; x <= this.maxBreadth + 1; x++) {
      let maxVertLength = 0;
      for (let y = 0; y <= this.maxDepth + 1; y++) {
        maxVertLength = Math.max(maxVertLength, this.map[y][x].vert.length);
      }

      for (let y = 0; y <= this.maxDepth + 1; y++) {
        this.map[y][x].vert.length = maxVertLength;
      }
    }
  }

  // Take the port at the specified block, side, and dir.
  takePort(block, side, dir, edge) {
    const info = this.getInfoAt(block.breadth, block.depth);
    info.ports[side][dir].push(edge);
  }

  getInfoAt(breadth, depth) {
    let x = breadth + 1;
    let y = depth + 1;
    return this.map[y][x];
  }

  // Get the actual position of the port for the given (block, side, dir, edge).
  getPortPos(block, side, dir, edge) {
    const info = this.getInfoAt(block.breadth, block.depth);
    const port = info.ports[side];
    const total = port.in.length + port.out.length;
    const BLOCK_MIN_WIDTH = 100;
    let index;

    if (side === "up") {
      if (dir === "in") {
        index = port.in.indexOf(edge);
      } else {
        index = port.in.length + port.out.indexOf(edge);
      }
    } else {
      if (dir === "out") {
        index = port.out.indexOf(edge);
      } else {
        index = port.out.length + port.in.indexOf(edge);
      }
    }

    const x =
          info.blockLeft + info.blockWidth / 2 - BLOCK_MIN_WIDTH / 2 +
          PORT_MARGIN + (BLOCK_MIN_WIDTH - PORT_MARGIN * 2) * (index + 1) / (total + 1);
    const y = info.blockTop + (side === "down" ? info.block.height : 0);

    return [x, y];
  }

  // Take the horizontal gap's slots at given depth by the edge,
  // for the [breadthFirst, breadthLast] range.
  takeHorzAt(breadthFirst, breadthLast, depth, edge) {
    if (breadthFirst > breadthLast) {
      const tmp = breadthLast;
      breadthLast = breadthFirst;
      breadthFirst = tmp;
    }

    let slot = 0;
    nextSlot: while (true) {
      for (let breadth = breadthFirst; breadth <= breadthLast; breadth++) {
        const info = this.getInfoAt(breadth, depth);
        if (info.horz[slot]) {
          slot++;
          continue nextSlot;
        }
      }
      break;
    }


    for (let breadth = breadthFirst; breadth <= breadthLast; breadth++) {
      const info = this.getInfoAt(breadth, depth);
      info.horz[slot] = edge;
    }
  }

  // Take the vertical gap's slots at given breadth by the edge,
  // for the [depthFirst, depthLast] range.
  takeVertAt(breadth, depthFirst, depthLast, edge) {
    if (depthFirst > depthLast) {
      const tmp = depthLast;
      depthLast = depthFirst;
      depthFirst = tmp;
    }

    let slot = 0;
    nextSlot: while (true) {
      for (let depth = depthFirst; depth <= depthLast; depth++) {
        const info = this.getInfoAt(breadth, depth);
        if (info.vert[slot]) {
          slot++;
          continue nextSlot;
        }
      }
      break;
    }

    for (let depth = depthFirst; depth <= depthLast; depth++) {
      const info = this.getInfoAt(breadth, depth);
      info.vert[slot] = edge;
    }
  }

  // Get the horizontal gap slot's Y coord.
  getHorzAt(breadth, depth, edge) {
    const info = this.getInfoAt(breadth, depth);
    const total = info.horz.length;
    const index = info.horz.indexOf(edge);
    return info.blockTop + info.blockHeight + CROSS_MARGIN +
      (info.crossHeight - CROSS_MARGIN * 2) * (index + 1) / (total + 1);
  }

  // Get the vertical gap slot's X coord.
  getVertAt(breadth, depth, edge) {
    const info = this.getInfoAt(breadth, depth);
    const total = info.vert.length;
    const index = info.vert.indexOf(edge);
    return info.blockLeft + info.blockWidth + CROSS_MARGIN +
      (info.crossWidth - CROSS_MARGIN * 2) * (index + 1) / (total + 1);
  }

  // Get the size of the entire grid.
  getWidth() {
    const info = this.map[1][this.maxBreadth + 1];
    return info.blockLeft + info.blockWidth + info.crossWidth;
  }
  getHeight() {
    const info = this.map[this.maxDepth + 1][1];
    return info.blockTop + info.blockHeight + info.crossHeight;
  }
}

// The gap between items in a row, and beside a long edge's dummy (see
// `RowLayout`), and a dummy's width.
const ROW_GAP = 24;
const ROW_DUMMY_GAP = 8;
const ROW_DUMMY_WIDTH = 2;
// How many passes order the rows, and place their items, and how much a
// long edge's dummies count for, when placing them, against the items.
const ROW_ORDER_PASSES = 8;
const ROW_PLACE_PASSES = 6;
const ROW_DUMMY_WEIGHT = 4;
// The space between a block's ports, and between the tracks of the gaps
// between rows, and the gaps' margins (above and below their tracks).
const PORT_SPACING = 8;
const TRACK_GAP = CROSS_GAP;
const TRACK_MARGIN = CROSS_MARGIN;

// The median of `values` (the mean of the middle two, if they're even).
function median(values) {
  const sorted = values.slice().sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

/**
 * A layout of items in rows, each item as wide as it is, rather than in
 * the grid's columns (`GridLayout`, whose columns are as wide as their widest
 * items, and rows as tall as their tallest, so that a big item, ex: a
 * cluster's box, see graph-clusters.js, stretches its row and its column),
 * in the spirit of iongraph's (see the blog post on its web version, 2025):
 *
 * - The rows are graph.js's depths (see `Graph.calculateDepth`), and an edge
 *   between rows which aren't adjacent has a dummy in each row between them,
 *   which the next steps place like an item, so that it goes around the
 *   items in the way, and straight if it can.
 * - The rows' order is their items' neighbors' median positions, a few
 *   passes down and up, keeping the order with the fewest crossings.
 * - Each row's items are placed as near as they can be (in that order, apart
 *   by their gaps) to their neighbors' median positions in the row above, or
 *   below, which is a weighted isotonic regression (see `placeRow`), a fixed
 *   number of passes down and up (as iongraph's are, rather than to a fixed
 *   point), the dummies counting for more, which straightens long edges.
 * - Each block's ports are on its top and bottom, each one as near as it can
 *   be to the other end's (so an edge between blocks over each other is
 *   straight), and each gap between rows has tracks for the edges' parts
 *   across it, railroad style: those going right one way, those going left
 *   another (in the orders in which they don't cross each other), and those
 *   between blocks in the same row, which go down and back up, nearest it.
 *
 * `items` have a `width` and a `height`, and get a `layoutLeft` and a
 * `layoutTop`; `edges` have a `pred` and a `succ` (items), and get their
 * `layoutPoints` (corners, from the end in the upper row to the other) and
 * whether they're drawn `upward` (from their succ to their pred).  The items
 * are `margin` from the layout's edges.
 */
class RowLayout {
  constructor(items, edges, { margin = CROSS_MARGIN } = {}) {
    this.items = items;
    this.edges = edges.filter(edge => edge.pred !== edge.succ);
    this.margin = margin;
  }

  // Lay out the items and edges; returns the layout's size.
  run() {
    this.assignRows();
    this.addDummies();
    this.orderRows();
    this.placeRows();
    this.assignPorts();
    this.assignTracks();
    this.setPositions();
    return this.size;
  }

  assignRows() {
    this.nodes = this.items.map(item => ({
      item, width: item.width, height: item.height, preds: [], succs: [], depth: -1,
      ups: [], downs: [], upSegs: [], downSegs: [],
    }));
    this.nodeOf = new Map(this.nodes.map(node => [node.item, node]));
    for (const edge of this.edges) {
      const pred = this.nodeOf.get(edge.pred);
      const succ = this.nodeOf.get(edge.succ);
      pred.succs.push(succ);
      succ.preds.push(pred);
    }
    const context = { blocks: this.nodes };
    Graph.prototype.calculateDepth.call(context);
    Graph.prototype.compressDepth.call(context);
    const rows = 1 + Math.max(0, ...this.nodes.map(node => node.depth));
    this.rows = Array.from({ length: rows }, () => []);
    for (const node of this.nodes) {
      this.rows[node.depth].push(node);
    }
  }

  // Each edge's segments between adjacent rows, through its dummies.
  addDummies() {
    this.segments = [];
    this.sameRow = [];
    const link = (top, bottom, edge) => {
      const segment = { top, bottom, edge, fromX: 0, toX: 0, track: -1 };
      top.downs.push(bottom);
      bottom.ups.push(top);
      top.downSegs.push(segment);
      bottom.upSegs.push(segment);
      this.segments.push(segment);
      return segment;
    };
    for (const edge of this.edges) {
      let top = this.nodeOf.get(edge.pred);
      let bottom = this.nodeOf.get(edge.succ);
      edge.upward = top.depth > bottom.depth;
      if (edge.upward) {
        [top, bottom] = [bottom, top];
      }
      edge.rowSegments = [];
      if (top.depth === bottom.depth) {
        // (Down from the pred, along the gap under the row, and back up.)
        const segment = { top, bottom, edge, fromX: 0, toX: 0, track: -1, sameRow: true };
        top.downSegs.push(segment);
        bottom.downSegs.push(segment);
        this.sameRow.push(segment);
        edge.rowSegments.push(segment);
        continue;
      }
      let above = top;
      for (let depth = top.depth + 1; depth < bottom.depth; depth++) {
        const dummy = {
          dummy: true, edge, width: ROW_DUMMY_WIDTH, height: 0, depth,
          ups: [], downs: [], upSegs: [], downSegs: [],
        };
        this.rows[depth].push(dummy);
        edge.rowSegments.push(link(above, dummy, edge));
        above = dummy;
      }
      edge.rowSegments.push(link(above, bottom, edge));
    }
  }

  indexRows() {
    for (const row of this.rows) {
      row.forEach((node, i) => { node.pos = i; });
    }
  }

  // The crossings of the segments between adjacent rows.
  crossings() {
    let count = 0;
    for (const row of this.rows) {
      const segments = row.flatMap(node => node.downSegs.filter(segment => !segment.sameRow));
      for (let i = 0; i < segments.length; i++) {
        const a = segments[i];
        for (let j = i + 1; j < segments.length; j++) {
          const b = segments[j];
          if ((a.top.pos - b.top.pos) * (a.bottom.pos - b.bottom.pos) < 0) {
            count++;
          }
        }
      }
    }
    return count;
  }

  orderRows() {
    this.indexRows();
    let best = this.rows.map(row => row.slice());
    let fewest = this.crossings();
    for (let pass = 0; pass < ROW_ORDER_PASSES && fewest > 0; pass++) {
      const down = pass % 2 === 0;
      for (let k = 1; k < this.rows.length; k++) {
        const row = this.rows[down ? k : this.rows.length - 1 - k];
        const key = new Map(row.map(node => {
          const neighbors = down ? node.ups : node.downs;
          return [node, neighbors.length ? median(neighbors.map(other => other.pos)) : node.pos];
        }));
        row.sort((a, b) => key.get(a) - key.get(b) || a.pos - b.pos);
        row.forEach((node, i) => { node.pos = i; });
      }
      const count = this.crossings();
      if (count < fewest) {
        fewest = count;
        best = this.rows.map(row => row.slice());
      }
    }
    this.rows = best;
    this.indexRows();
  }

  // How far apart the centers of adjacent nodes in a row must be.
  separation(a, b) {
    const gap = a.dummy || b.dummy ? ROW_DUMMY_GAP : ROW_GAP;
    return a.width / 2 + gap + b.width / 2;
  }

  /**
   * Place `row`'s nodes' centers (`x`) as near as they can be to `desired`
   * (by `weights`, the least weighted squared distance), in their order and
   * apart by their separations: with `y` the centers less their minimal
   * offsets from the first, the `y`s must be in order, which pooling
   * adjacent violators solves exactly.
   */
  placeRow(row, desired, weights) {
    const offsets = [0];
    for (let i = 1; i < row.length; i++) {
      offsets.push(offsets[i - 1] + this.separation(row[i - 1], row[i]));
    }
    const pools = [];
    for (let i = 0; i < row.length; i++) {
      let pool = { sum: weights[i] * (desired[i] - offsets[i]), weight: weights[i], start: i, end: i };
      while (pools.length && pools[pools.length - 1].sum / pools[pools.length - 1].weight > pool.sum / pool.weight) {
        const before = pools.pop();
        pool = { sum: before.sum + pool.sum, weight: before.weight + pool.weight, start: before.start, end: pool.end };
      }
      pools.push(pool);
    }
    for (const pool of pools) {
      for (let i = pool.start; i <= pool.end; i++) {
        row[i].x = pool.sum / pool.weight + offsets[i];
      }
    }
  }

  placeRows() {
    for (const row of this.rows) {
      let x = 0;
      row.forEach((node, i) => {
        x += i ? this.separation(row[i - 1], node) : node.width / 2;
        node.x = x;
      });
    }
    const passes = [];
    for (let i = 0; i < ROW_PLACE_PASSES; i++) {
      passes.push(true, false);
    }
    passes.push(true);
    for (const down of passes) {
      for (let k = 1; k < this.rows.length; k++) {
        const row = this.rows[down ? k : this.rows.length - 1 - k];
        const neighbors = row.map(node => (down ? node.ups : node.downs));
        const desired = row.map((node, i) =>
          (neighbors[i].length ? median(neighbors[i].map(other => other.x)) : node.x));
        // (Nodes with no neighbors that way stay about where they are.)
        const weights = row.map((node, i) =>
          (node.dummy ? ROW_DUMMY_WEIGHT : 1) * (neighbors[i].length ? 1 : 0.1));
        this.placeRow(row, desired, weights);
      }
    }
    // (On whole pixels, which the gaps leave room for.)
    const left = Math.min(...this.nodes.map(node => node.x - node.width / 2));
    for (const row of this.rows) {
      for (const node of row) {
        node.x = Math.round(node.x + this.margin - left);
      }
    }
  }

  /**
   * Place the ports of `node`'s side whose `ends` (each `{ desired, set }`)
   * are those of its segments: in the order of their other ends, each as
   * near as it can be to its other end's (`desired`), within the block,
   * PORT_SPACING apart (or evenly, if they don't fit).
   */
  placePorts(node, ends) {
    if (node.dummy) {
      for (const end of ends) {
        end.set(node.x);
      }
      return;
    }
    const low = node.x - node.width / 2 + PORT_MARGIN;
    const high = node.x + node.width / 2 - PORT_MARGIN;
    ends.sort((a, b) => a.desired - b.desired);
    if (high <= low || (ends.length - 1) * PORT_SPACING > high - low) {
      ends.forEach((end, i) => end.set(high <= low ? node.x : Math.round(low + (high - low) * (i + 0.5) / ends.length)));
      return;
    }
    const xs = ends.map(end => clamp(end.desired, low, high));
    for (let i = 1; i < xs.length; i++) {
      xs[i] = Math.max(xs[i], xs[i - 1] + PORT_SPACING);
    }
    for (let i = xs.length - 1; i >= 0; i--) {
      xs[i] = Math.min(xs[i], i === xs.length - 1 ? high : xs[i + 1] - PORT_SPACING);
    }
    ends.forEach((end, i) => end.set(Math.round(xs[i])));
  }

  // The ports, row by row from the top: a block's top ones as near as they
  // can be to their segments' other ends' ports, its bottom ones to their
  // other ends.
  assignPorts() {
    for (const row of this.rows) {
      for (const node of row) {
        this.placePorts(node, node.upSegs.map(segment => ({
          desired: segment.fromX, set: x => { segment.toX = x; },
        })));
      }
      for (const node of row) {
        this.placePorts(node, node.downSegs.map(segment => {
          const fromTop = segment.top === node;
          const other = segment.sameRow ? (fromTop ? segment.bottom : segment.top) : segment.bottom;
          return {
            desired: other.x,
            set: fromTop ? x => { segment.fromX = x; } : x => { segment.toX = x; },
          };
        }));
      }
    }
  }

  /**
   * The tracks of each gap under a row, for its segments which aren't
   * straight: those between blocks in the row nearest it, the shortest
   * first, then those going right (the one starting furthest right first,
   * which then doesn't cross the others), then those going left (the one
   * starting furthest left first).  Each one's track is the one after those
   * of the segments before it whose spans overlap its, so that they stay in
   * that order, but tracks are shared by segments which don't overlap.
   */
  assignTracks() {
    this.tracks = this.rows.map(() => 0);
    for (const [depth, row] of this.rows.entries()) {
      // (Those between blocks in the row are in both blocks' `downSegs`.)
      const segments = row.flatMap(node => node.downSegs.filter(segment => segment.top === node));
      const span = segment => [Math.min(segment.fromX, segment.toX), Math.max(segment.fromX, segment.toX)];
      const groups = [
        segments.filter(segment => segment.sameRow)
          .sort((a, b) => (span(a)[1] - span(a)[0]) - (span(b)[1] - span(b)[0])),
        segments.filter(segment => !segment.sameRow && segment.toX - segment.fromX >= 1)
          .sort((a, b) => b.fromX - a.fromX),
        segments.filter(segment => !segment.sameRow && segment.fromX - segment.toX >= 1)
          .sort((a, b) => a.fromX - b.fromX),
      ];
      let base = 0;
      for (const group of groups) {
        const placed = [];
        let count = 0;
        for (const segment of group) {
          const [low, high] = span(segment);
          let track = 0;
          for (const other of placed) {
            const [otherLow, otherHigh] = span(other);
            if (low < otherHigh + PORT_SPACING && otherLow < high + PORT_SPACING) {
              track = Math.max(track, other.track - base + 1);
            }
          }
          segment.track = base + track;
          count = Math.max(count, track + 1);
          placed.push(segment);
        }
        base += count;
      }
      this.tracks[depth] = base;
    }
  }

  setPositions() {
    // The rows' tops, and their gaps' (under them) heights.
    const tops = [];
    const heights = this.rows.map(row => Math.max(0, ...row.map(node => node.height)));
    const gapHeight = depth => {
      const tracks = this.tracks[depth];
      if (depth === this.rows.length - 1 && tracks === 0) {
        return 0;
      }
      return TRACK_MARGIN * 2 + Math.max(0, tracks - 1) * TRACK_GAP;
    };
    let y = this.margin;
    for (const [depth] of this.rows.entries()) {
      tops.push(y);
      y += heights[depth] + gapHeight(depth);
    }
    const trackY = (depth, track) => {
      const top = tops[depth] + heights[depth];
      return this.tracks[depth] === 1
        ? top + gapHeight(depth) / 2
        : top + TRACK_MARGIN + track * TRACK_GAP;
    };

    for (const node of this.nodes) {
      node.item.layoutLeft = node.x - node.width / 2;
      node.item.layoutTop = tops[node.depth];
    }
    // A node's bottom (a dummy's, its row's), and its top.
    const bottomOf = node => (node.dummy ? tops[node.depth] + heights[node.depth] : tops[node.depth] + node.height);
    for (const edge of this.edges) {
      const points = [];
      const add = (x, y) => {
        const last = points[points.length - 1];
        if (!last || last.x !== x || last.y !== y) {
          points.push({ x, y });
        }
      };
      for (const segment of edge.rowSegments) {
        const depth = segment.top.depth;
        if (segment.sameRow) {
          const across = trackY(depth, segment.track);
          add(segment.fromX, bottomOf(segment.top));
          add(segment.fromX, across);
          add(segment.toX, across);
          add(segment.toX, bottomOf(segment.bottom));
          continue;
        }
        // (A straight segment's ends can be a little apart, ex: its ports'
        // on blocks whose sides are; it's drawn at its upper end's.)
        const x = points.length ? points[points.length - 1].x : segment.fromX;
        add(x, bottomOf(segment.top));
        if (segment.track >= 0) {
          const across = trackY(depth, segment.track);
          add(x, across);
          add(segment.toX, across);
        }
        add(segment.track >= 0 ? segment.toX : x, tops[segment.bottom.depth]);
      }
      // (Without the corners which aren't, ex: through dummies.)
      edge.layoutPoints = points.filter((point, i) => {
        if (i === 0 || i === points.length - 1) {
          return true;
        }
        const [before, after] = [points[i - 1], points[i + 1]];
        return !((before.x === point.x && point.x === after.x) || (before.y === point.y && point.y === after.y));
      });
    }
    const right = Math.max(0, ...this.nodes.map(node => node.x + node.width / 2));
    this.size = { x: right + this.margin, y: y + this.margin };
  }
}

// Construct a graph for the given input data.
// The graph of `input` (its `nodes` and `edges`) in `container`, panned and
// zoomed in `viewport`.  A node is a symbol, whose block shows its pretty
// identifier (by SYM_INFO), or an object with its `sym` and `pretty`, and other
// data for `options.renderBlock(block, node)`, which then fills its block's
// DOM node (ex: a try push's canvas's cards of code; see rev-canvas.js), and
// `setInput` lays out another input in the same viewport.
class Graph {
  constructor(viewport, container, input, sources, targets, extraList, options = {}) {
    this.viewport = viewport;
    this.graphContainer = container;
    this.extraList = extraList;
    this.options = options;

    const viewportRect = this.viewport.getBoundingClientRect();
    this.viewportSize = {
      x: viewportRect.width,
      y: viewportRect.height
    };

    this.zoom = 1;
    this.translation = { x: 0, y: 0 };

    this.appliedZoom = 1;
    this.appliedTranslation = { x: 0, y: 0 };

    this.blocks = [];
    this.edges = [];

    // (The input, and the nodes left out of it, for `setHidden`.)
    this.input = input;
    this.sources = sources;
    this.targets = targets;
    this.hiddenSyms = options.hiddenSyms || new Set();
    this.removedPretties = new Set();

    this.createBlocksAndEdges(this.shownInput(), sources, targets);

    this.render();

    this.needInitialFitToViewport = true;
    this.addEventListeners();
    this.initialFitToViewport();
    this.addButtons();
  }

  render() {
    // (Grouped in clusters, with graph-clusters.js; a prototype.)
    if (this.options.clusterBy && typeof this.renderClustered == "function") {
      this.renderClustered();
      return;
    }
    this.graphContainer.replaceChildren();

    this.resetBlockAndEdgeState();

    // The blocks with edges are laid out in the grid, and those with none
    // beside it (see `placeIsolatedBlocks`), rather than in its first row,
    // where they made it as wide as there are of them, and moved the blocks
    // with edges apart.
    const allBlocks = this.blocks;
    const linked = new Set(this.edges.flatMap(edge => [edge.pred, edge.succ]));
    this.blocks = allBlocks.filter(block => linked.has(block));
    this.isolatedBlocks = allBlocks.filter(block => !linked.has(block));
    let gridSize = { x: 0, y: 0 };
    this.rowLayout = null;
    try {
      if (this.blocks.length > 0 && this.options.layout === "rows") {
        // (In rows rather than the grid; see `RowLayout`.)
        for (const block of this.blocks) {
          this.createBlockNode(block);
        }
        this.rowLayout = new RowLayout(this.blocks, this.edges.filter(edge => !edge.loopback));
        gridSize = this.rowLayout.run();
        for (const block of this.blocks) {
          block.left = block.layoutLeft;
          block.top = block.layoutTop;
          block.node.style.left = block.left + "px";
          block.node.style.top = block.top + "px";
        }
      } else if (this.blocks.length > 0) {
        this.calculateDepth();

        this.compressDepth();

        this.calculateBreadth();

        this.calculateEdgeDirection();

        this.calculateDimension();

        this.layout = new GridLayout(this.maxBreadth, this.maxDepth);

        this.takePorts();

        this.layout.sortPorts();

        this.createBlockNodes();

        this.calculateEdgePath();

        this.layout.calculatePosition();

        this.setBlockPosition();

        gridSize = { x: this.layout.getWidth(), y: this.layout.getHeight() };
      }
    } finally {
      this.blocks = allBlocks;
    }
    const isolatedSize = this.placeIsolatedBlocks(gridSize);
    this.size = {
      x: Math.max(gridSize.x, isolatedSize.x),
      y: Math.max(gridSize.y, isolatedSize.y),
    };

    this.createSVGNode();

    this.createEdgeNodes();

    this.createLoopEdgeNodes();

    // Re-populate interaction data
    this.extraList.length = 0;
    this.populateExtra(this.extraList);
  }

  // Lay out another input, keeping the pan and zoom.
  setInput(input, sources = [], targets = []) {
    this.input = input;
    this.sources = sources;
    this.targets = targets;
    this.blocks = [];
    this.edges = [];
    this.createBlocksAndEdges(this.shownInput(), sources, targets);
    this.render();
  }

  // The input without the nodes of `hiddenSyms` (and the edges with them).
  shownInput() {
    if (this.hiddenSyms.size == 0) {
      return this.input;
    }
    const symOf = node => (typeof node === "string" ? node : node.sym);
    const nodes = this.input.nodes.filter(node => !this.hiddenSyms.has(symOf(node)));
    const shown = new Set(nodes.map(symOf));
    const edges = this.input.edges.filter(edge => shown.has(edge.from) && shown.has(edge.to));
    return { ...this.input, nodes, edges };
  }

  // Lay out the input without the nodes of `hiddenSyms` (ex: those whose
  // facets' values the navigation panel's facets leave out; see
  // query_results/graph_input.liquid), and those removed (see
  // `removeNodeByPrettyName`), fitting it to the viewport.  (The nodes
  // which are then unreachable stay, since the facets' counts are of the
  // nodes they leave in.)
  setHidden(hiddenSyms) {
    this.hiddenSyms = hiddenSyms;
    this.blocks = [];
    this.edges = [];
    this.createBlocksAndEdges(this.shownInput(), this.sources, this.targets);
    for (const pretty of this.removedPretties) {
      this.dropBlockByPrettyName(pretty);
    }
    if (this.removedPretties.size > 0) {
      this.pruneUnreachableNodes();
    }
    this.render();
    this.fitToViewport(1);
  }

  resetBlockAndEdgeState() {
    for (const block of this.blocks) {
      block.depth = -1;
      block.breadth = -1;
    }

    for (const edge of this.edges) {
      edge.path = [];
    }
  }

  removeNodeByPrettyName(prettyName) {
    if (!this.dropBlockByPrettyName(prettyName)) {
      return;
    }
    this.removedPretties.add(prettyName);

    this.pruneUnreachableNodes();

    this.render(false);
  }

  // Remove a block, and its edges; returns whether it was there.
  dropBlockByPrettyName(prettyName) {
    const blockToRemove = this.blocks.find(b => b.pretty === prettyName);
    if (!blockToRemove) {
      return false;
    }

    const blockIdToRemove = blockToRemove.id;

    this.blocks = this.blocks.filter(block => block.id !== blockIdToRemove);

    this.edges = this.edges.filter(edge => {
      return edge.pred.id !== blockIdToRemove && edge.succ.id !== blockIdToRemove;
    });

    for (const block of this.blocks) {
      block.preds = block.preds.filter(p => p.id !== blockIdToRemove);
      block.succs = block.succs.filter(s => s.id !== blockIdToRemove);
    }
    return true;
  }

  pruneUnreachableNodes() {
    const sources = this.blocks.filter(b => b.isSource);
    const targets = this.blocks.filter(b => b.isTarget);

    const searchInput = document.getElementById("query");
    const queryStr = searchInput ? searchInput.value : (new URL(window.location.href)).searchParams.get("q") || "";
    const undirectedNames = [];
    const regex = /calls-between:(?:'([^']+)'|([^\s]+))/g;
    let match;
    while ((match = regex.exec(queryStr)) !== null) {
      undirectedNames.push(match[1] || match[2]);
    }
    const undirectedAnchors = this.blocks.filter(b => undirectedNames.includes(b.pretty));

    const focalNodes = [...new Set([...sources, ...targets, ...undirectedAnchors])];

    if (focalNodes.length === 0) {
      const isolatedIds = new Set(
        this.blocks.filter(b => b.preds.length === 0 && b.succs.length === 0 && !b.loopback).map(b => b.id)
      );
      if (isolatedIds.size > 0) {
        this.blocks = this.blocks.filter(b => !isolatedIds.has(b.id));
      }
      return;
    }

    const getReachable = (startNodes, direction) => {
      const visited = new Set(startNodes.map(b => b.id));
      const queue = startNodes.slice();
      while (queue.length > 0) {
        const curr = queue.shift();
        for (const next of curr[direction]) {
          if (!visited.has(next.id)) {
            visited.add(next.id);
            queue.push(next);
          }
        }
      }
      return visited;
    };

    const isCallsBetweenDirected = sources.length > 0 && targets.length > 0 && !sources.some(s => targets.includes(s));
    const isCallsBetweenUndirected = undirectedAnchors.length > 1;

    let validIds = new Set();

    if (isCallsBetweenDirected) {
      const fromSources = getReachable(sources, "succs");
      const toTargets = getReachable(targets, "preds");
      validIds = new Set([...fromSources].filter(id => toTargets.has(id)));
    } else if (isCallsBetweenUndirected) {
      const fromAnchors = getReachable(undirectedAnchors, "succs");
      const toAnchors = getReachable(undirectedAnchors, "preds");
      validIds = new Set([...fromAnchors].filter(id => toAnchors.has(id)));
    } else {
      getReachable(focalNodes, "succs").forEach(id => validIds.add(id));
      getReachable(focalNodes, "preds").forEach(id => validIds.add(id));
    }

    this.blocks = this.blocks.filter(b => validIds.has(b.id));
    this.edges = this.edges.filter(e => validIds.has(e.pred.id) && validIds.has(e.succ.id));

    for (const block of this.blocks) {
      block.preds = block.preds.filter(p => validIds.has(p.id));
      block.succs = block.succs.filter(s => validIds.has(s.id));
    }
  }

  // Updates the search input and URL query string without reloading the page.
  updateQueryForIgnoredNode(prettyName) {
    const searchInput = document.getElementById("query");
    const url = new URL(window.location.href);
    let query = searchInput ? searchInput.value : url.searchParams.get("q");

    if (query) {
      // (As `Diagram.ignoreNode` adds it: escaped in the list, which is
      // quoted if it has to be, ex: for a web spec's term's pretty name.)
      let newIgnoreList = Diagram.escapeIgnoredNode(prettyName);

      const match = query.match(Diagram.queryTermRegExp("ignore-nodes"));
      if (match) {
        newIgnoreList = Diagram.unquoteQueryValue(match[1]) + "," + newIgnoreList;
      }

      query = query.replace(Diagram.queryTermRegExp("ignore-nodes", "g"), "");
      query += " ignore-nodes:" + Diagram.quoteQueryValue(newIgnoreList);

      if (searchInput) {
        searchInput.value = query.trim();
      }
      url.searchParams.set("q", query.trim());
      window.history.pushState({}, "", url);
    }
  }

  // Create data objects for blocks and edges.
  createBlocksAndEdges(input, sources, targets) {
    const symToBlock = new Map();
    const prettyToBlock = new Map();

    function splitPretty(pretty) {
      const m = pretty.match(/^(.+::)([^:]+)$/);
      if (!m) {
        return ["", pretty];
      }

      return [m[1], m[2]];
    }

    let nextBlockId = 0;
    for (const inputNode of input.nodes) {
      const sym = typeof inputNode === "string" ? inputNode : inputNode.sym;
      const pretty = typeof inputNode === "string" ? SYM_INFO[sym].pretty : inputNode.pretty;
      if (prettyToBlock.has(pretty)) {
        const block = prettyToBlock.get(pretty);
        block.syms.push(sym);
        symToBlock.set(sym, block);
        continue;
      }

      const contents = [];

      const [scope, text] = splitPretty(pretty);

      if (scope) {
        contents.push({ scope });
      }
      contents.push({ text });

      const isSource = sources.includes(sym);
      const isTarget = targets.includes(sym);

      const block = {
        syms: [sym],
        pretty,
        id: nextBlockId++,
        isSource,
        isTarget,

        preds: [],
        succs: [],
        loopback: null,

        contents,
        data: typeof inputNode === "string" ? null : inputNode,

        depth: -1,
        breadth: -1,
      };
      symToBlock.set(sym, block);
      prettyToBlock.set(pretty, block);

      this.blocks.push(block);
    }

    const keyToEdge = new Map();

    let nextEdgeId = 0;
    for (const { from: fromSym, to: toSym, kind, jumps, hovers } of input.edges) {
      const fromBlock = symToBlock.get(fromSym);
      const toBlock = symToBlock.get(toSym);

      const key = fromBlock.pretty + "\x01" + toBlock.pretty + "\x01" + kind;
      if (keyToEdge.has(key)) {
        const edge = keyToEdge.get(key);
        edge.jumps.push(...jumps);
        edge.hovers.push(...hovers);
        continue;
      }

      const loopback = fromBlock === toBlock;

      const edge = {
        syms: [fromSym + "->" + toSym],
        id: nextEdgeId++,
        pred: fromBlock,
        succ: toBlock,
        loopback,

        kind,
        jumps: jumps,
        hovers,

        predSide: "",
        succSide: "",
        predDir: "",
        succDir: "",
        flipped: false,
        path: [],
        length: 0,
      };
      keyToEdge.set(key, edge);

      if (loopback) {
        fromBlock.loopback = edge;
      } else {
        fromBlock.succs.push(toBlock);
        toBlock.preds.push(fromBlock);
      }

      this.edges.push(edge);
    }
  }

  // Calculate the depth that reflects the edges between the blocks.
  calculateDepth() {
    const propagate = roots => {
      const pending = roots.map(block => ({ block, path: [] }));
      while (pending.length > 0) {
        const { block, path } = pending.shift();
        for (const succBlock of block.succs) {
          if (succBlock.depth > block.depth) {
            continue;
          }
          if (path.includes(succBlock)) {
            continue;
          }
          succBlock.depth = block.depth + 1;
          pending.push({ block: succBlock, path: path.concat([block]) });
        }
      }
    };

    const roots = this.blocks.filter(b => b.preds.length === 0);
    for (const block of roots) {
      block.depth = 0;
    }
    propagate(roots);

    // The blocks no root reaches: those of a cycle with no way in (ex: a
    // trace's function and a callee calling it back), and what they reach.
    // The first of them is a root too, until all of them have depths (which
    // the other steps need: compressDepth looped forever without them).
    for (let block; (block = this.blocks.find(b => b.depth < 0)); ) {
      block.depth = 0;
      propagate([block]);
    }
  }

  // calculateDepth algorithm can create a depth with no blocks.
  // Shift those items up.
  compressDepth() {
    let remaining = this.blocks.slice();
    let destDepth = 0;
    let srcDepth = 0;
    // (Every block has a depth, from calculateDepth; but should one not, it
    // goes at the last depth rather than this looping forever.)
    const maxDepth = Math.max(0, ...remaining.map(b => b.depth));

    while (remaining.length > 0) {
      if (srcDepth > maxDepth) {
        for (const block of remaining) {
          block.depth = destDepth;
        }
        break;
      }
      const blocksInDepth = remaining.filter(b => b.depth === srcDepth);
      remaining = remaining.filter(b => b.depth !== srcDepth);
      if (blocksInDepth.length === 0) {
        srcDepth++;
        continue;
      }
      if (srcDepth !== destDepth) {
        for (const block of blocksInDepth) {
          block.depth = destDepth;
        }
      }
      srcDepth++;
      destDepth++;
    }
  }

  // Calculate the breadth-axis position for blocks, that reflects the
  // edges and clusters, and also make the edge straight as much as possible.
  calculateBreadth() {
    function splitBlocksPerDepth(remaining) {
      let depth = 0;
      const depthToBlocks = new Map();
      while (remaining.length > 0) {
        const blocksInDepth = remaining.filter(b => b.depth === depth);

        depthToBlocks.set(depth, blocksInDepth);
        remaining = remaining.filter(b => b.depth !== depth);
        depth++;
      }
      return depthToBlocks;
    };
    function sortBlocks(depthToBlocks) {
      for (const [_, blocksInDepth] of depthToBlocks) {
        for (const [index, block] of blocksInDepth.entries()) {
          block.tmpBreadth = index;
        }
      }

      for (const [_, blocksInDepth] of depthToBlocks) {
        for (const [index, block] of blocksInDepth.entries()) {
          let list = block.preds.map(b => b.tmpBreadth).concat(block.succs.map(b => b.tmpBreadth));
          if (list.length === 0) {
            continue;
          }
          block.tmpBreadth = list.reduce((a, b) => a + b, 0) / list.length;
        }
      }

      for (const [_, blocksInDepth] of depthToBlocks) {
        // (By their neighbors' mean breadth; a comparator returning a
        // boolean, as this did, doesn't order them.)
        blocksInDepth.sort((a, b) => a.tmpBreadth - b.tmpBreadth);
      }
    }
    function calculateFinalBreadth(depthToBlocks) {
      for (const [depth, blocksInDepth] of depthToBlocks) {
        let breadth = 0;
        for (const block of blocksInDepth) {
          if (block.preds.length > 0 &&
              block.preds.every(predBlock => predBlock.depth < block.depth)) {

            let minBreadth = -1;
            for (const predBlock of block.preds) {
              if (minBreadth === -1) {
                minBreadth = predBlock.breadth;
              } else {
                minBreadth = Math.min(minBreadth, predBlock.breadth);
              }
            }

            if (minBreadth > breadth) {
              breadth = minBreadth;
            }
          }

          block.breadth = breadth;
          breadth++;
        }
      }
    }

    const depthToBlocks = splitBlocksPerDepth(this.blocks.slice());

    for (let i = 0; i < 3; i++) {
      sortBlocks(depthToBlocks);
    }

    calculateFinalBreadth(depthToBlocks);
  }

  // Determine which side of the block the edge should connect to.
  calculateEdgeDirection() {
    for (const edge of this.edges) {
      if (edge.loopback) {
        continue;
      }

      // All edges goes from the small depth to the large depth.
      // If the edge goes to the opposite direction, it's flipped.
      if (edge.pred.depth > edge.succ.depth) {
        const tmp = edge.pred;
        edge.pred = edge.succ;
        edge.succ = tmp;
        edge.flipped = true;
        edge.predDir = "in";
        edge.succDir = "out";
      } else {
        edge.predDir = "out";
        edge.succDir = "in";
      }

      if (edge.pred.depth < edge.succ.depth) {
        edge.predSide = "down";
        edge.succSide = "up";
      } else {
        // If the edge connects the blocks at the same depth,
        // use the "down" port.
        //
        // +-------+         +-------+
        // | block |         | block |
        // +-------+         +-------+
        //    |                 ^
        //    |                 |
        //    +-----------------+
        //
        edge.predSide = "down";
        edge.succSide = "down";
      }

      edge.length = Math.abs(edge.pred.breadth - edge.succ.breadth) +
        Math.abs(edge.pred.depth - edge.succ.depth);
    }
  }

  calculateDimension() {
    let maxDepth = 0;
    let maxBreadth = 0;

    for (const block of this.blocks) {
      maxDepth = Math.max(maxDepth, block.depth);
      maxBreadth = Math.max(maxBreadth, block.breadth);
    }

    this.maxDepth = maxDepth;
    this.maxBreadth = maxBreadth;
  }

  // Let all edges take the ports for the connecting blocks.
  takePorts() {
    for (const edge of this.edges) {
      if (edge.loopback) {
        continue;
      }

      this.layout.takePort(edge.pred, edge.predSide, edge.predDir, edge);
      this.layout.takePort(edge.succ, edge.succSide, edge.succDir, edge);
    }
  }

  // Create the DOM node for each block in the grid.
  createBlockNodes() {
    for (const block of this.blocks) {
      this.createBlockNode(block);
      this.layout.addBlock(block.breadth, block.depth, block);
    }
  }

  // Create a block's DOM node, measuring it.
  createBlockNode(block) {
    {
      const node = document.createElement("div");
      node.id = "Gidn" + block.id;
      node.setAttribute("data-symbols", block.syms.join(","));
      node.classList.add("interactive-graph-block");
      if (block.isSource) {
        node.classList.add("interactive-graph-block-source");
      }
      if (block.isTarget) {
        node.classList.add("interactive-graph-block-target");
      }
      if (this.options.renderBlock) {
        this.options.renderBlock(block, node);
      } else {
        // (The cluster it's alone of, if it's been taken out of it; see
        // graph-clusters.js.)
        if (block.tag) {
          const tag = document.createElement("div");
          tag.classList.add("interactive-graph-block-tag");
          tag.append(block.tag);
          node.append(tag);
        }
        for (const line of block.contents) {
          if (line.scope) {
            const box = document.createElement("div");
            box.classList.add("interactive-graph-block-scope");
            box.append(line.scope);
            node.append(box);
          }
          if (line.text) {
            const box = document.createElement("div");
            box.classList.add("interactive-graph-block-text");
            box.append(line.text);
            node.append(box);
          }
        }
      }
      this.graphContainer.append(node);

      // (Its own size, not its zoomed one.)
      block.width = node.offsetWidth;
      block.height = node.offsetHeight;

      block.node = node;
    }
  }

  // Place the blocks with no edges (`isolatedBlocks`) beside the grid (of
  // `gridSize`), in columns to its right, as tall as the grid, or taller if
  // that fits the whole graph in the viewport better: the height whose
  // graph the initial fit shows biggest (see `fitToViewport`), the grid's
  // when they tie, since the graph is easier to read, and scroll, down than
  // across.  Returns the size of the grid with them.
  placeIsolatedBlocks(gridSize) {
    const blocks = this.isolatedBlocks;
    if (blocks.length === 0) {
      return { x: 0, y: 0 };
    }
    for (const block of blocks) {
      this.createBlockNode(block);
    }
    const left = gridSize.x > 0 ? gridSize.x + ISOLATED_GAP : 0;
    // The columns of blocks no taller than `height` (but one block's), and
    // the size of the graph with them.
    const pack = height => {
      const columns = [];
      let column = null;
      for (const block of blocks) {
        if (!column || (column.height > 0 && column.height + ISOLATED_GAP + block.height > height)) {
          column = { blocks: [], width: 0, height: 0 };
          columns.push(column);
        }
        column.height += (column.blocks.length ? ISOLATED_GAP : 0) + block.height;
        column.width = Math.max(column.width, block.width);
        column.blocks.push(block);
      }
      const width = left + columns.reduce((sum, c) => sum + c.width, 0) +
        ISOLATED_GAP * (columns.length - 1);
      const tallest = Math.max(...columns.map(c => c.height));
      return { columns, size: { x: width, y: Math.max(gridSize.y, tallest) } };
    };
    const base = Math.max(gridSize.y, ...blocks.map(block => block.height));
    const total = blocks.reduce((sum, block) => sum + block.height + ISOLATED_GAP, 0);
    const fit = ({ size }) => Math.min(this.viewportSize.x / size.x || 1,
                                       this.viewportSize.y / size.y || 1, 1);
    let best = pack(base);
    for (let height = base * 1.25; height < total + base; height *= 1.25) {
      const packed = pack(height);
      if (fit(packed) > fit(best) * 1.01) {
        best = packed;
      }
    }
    let x = left;
    for (const column of best.columns) {
      let y = 0;
      for (const block of column.blocks) {
        block.left = x + (column.width - block.width) / 2;
        block.top = y;
        block.node.style.left = block.left + "px";
        block.node.style.top = block.top + "px";
        y += block.height + ISOLATED_GAP;
      }
      x += column.width + ISOLATED_GAP;
    }
    return best.size;
  }

  // For the mouse interaction, populate the GRAPH_EXTRA, with the
  // same format as the current server-side layout.
  //
  // TODO: This could be simplified by directly using the data.
  populateExtra(extraList) {
    const extra = {
      nodes: {},
      edges: {},
    };

    for (const block of this.blocks) {
      const in_nodes = [];
      const out_nodes = [];

      for (const predBlock of block.preds) {
        // TODO: reflect hover classes.
        in_nodes.push(["Gidn" + predBlock.id, []]);
      }
      for (const succBlock of block.succs) {
        out_nodes.push(["Gidn" + succBlock.id, []]);
      }

      extra.nodes["Gidn" + block.id] = {
        in_edges: [],
        out_edges: [],
        in_nodes,
        out_nodes,
      };
    }

    function mergeJumps(jumps) {
      jumps = [...new Set(jumps)];

      if (jumps.length === 0) {
        return undefined;
      }

      const m = jumps[0].match(/^(.+)#(.+)$/);
      if (!m) {
        return jumps[0];
      }

      const path = m[1];
      const lines = [m[2]];

      for (const jump of jumps.slice(1)) {
        const m = jumps[0].match(/^(.+)#(.+)$/);
        if (!m) {
          continue;
        }

        if (path !== m[1]) {
          continue;
        }
        lines.push(m[2]);
      }

      return path + "#" + lines.join(",");
    }

    for (const edge of this.edges) {
      extra.nodes["Gidn" + edge.pred.id].out_edges.push("Gide" + edge.id);
      extra.nodes["Gidn" + edge.succ.id].in_edges.push("Gide" + edge.id);

      extra.edges["Gide" + edge.id] = {
        jump: mergeJumps(edge.jumps),
      };
    }

    extraList.push(extra);
  }

  // Check if there's any block at (breadth, depth1) to (breadth, depth2)
  // range.
  //
  // If there's no block in the range, an edge can go through that area,
  // without detouring.
  hasBlock(breadth, depth1, depth2) {
    while (depth1 !== depth2 - 1) {
      depth2--;
      if (this.layout.getInfoAt(breadth, depth2).block) {
        return true;
      }
    }
    return false;
  }

  // Calculate the edge's path as a sequence of the corner's X coord or Y coord.
  calculateEdgePath() {
    for (const edge of this.edges) {
      const predDepth = edge.pred.depth;
      const predBreadth = edge.pred.breadth;
      const succDepth = edge.succ.depth;
      const succBreadth = edge.succ.breadth;

      let depth = predDepth;
      let breadth = predBreadth;

      const noBlock = edge.succSide === "up" &&
            !this.hasBlock(succBreadth, predDepth, succDepth);

      if (noBlock || depth === succDepth - 1) {
        // | pred |
        // +------+
        //   |
        //   +--------+
        //            |
        //            v
        //  ...     +------+
        //          | succ |

        this.layout.takeHorzAt(breadth, succBreadth, depth, edge);
        edge.path.push([breadth, depth, "horz"]);
      } else if (breadth === succBreadth) {
        // | pred |
        // +------+
        //   |
        //   +-------+
        //           |
        //   ...     |
        //           |
        //   +-------+
        //   |
        //   v
        // +------+
        // | succ |
        //

        this.layout.takeHorzAt(breadth, breadth, depth, edge);
        edge.path.push([breadth, depth, "horz"]);

        this.layout.takeVertAt(breadth, depth, succDepth - 1, edge);
        edge.path.push([breadth, depth, "vert"]);

        depth = succDepth - 1;

        this.layout.takeHorzAt(breadth, breadth, depth, edge);
        edge.path.push([breadth, depth, "horz"]);
      } else if (breadth < succBreadth) {
        if (edge.succSide === "up") {
          // | pred |
          // +------+
          //   |
          //   +-------+
          //           |
          //   ...     |
          //           |
          //           +---+
          //               |
          //               v
          //             +------+
          //             | succ |

          this.layout.takeHorzAt(breadth, succBreadth - 1, depth, edge);
          edge.path.push([breadth, depth, "horz"]);

          breadth = succBreadth - 1;

          this.layout.takeVertAt(breadth, depth, succDepth - 1, edge);
          edge.path.push([breadth, depth, "vert"]);

          depth = succDepth - 1;

          this.layout.takeHorzAt(breadth, succBreadth, depth, edge);
          edge.path.push([succBreadth, depth, "horz"]);
        } else {
          // | pred |     | succ |
          // +------+     +------+
          //    |            |
          //    +------------+

          this.layout.takeHorzAt(breadth, succBreadth, depth, edge);
          edge.path.push([breadth, depth, "horz"]);
        }
      } else {
        //             | pred |
        //             +------+
        //               |
        //           +---+
        //           |
        //   ...     |
        //           |
        //   +-------+
        //   |
        //   v
        // +------+
        // | succ |

        this.layout.takeHorzAt(breadth, succBreadth, depth, edge);
        edge.path.push([breadth, depth, "horz"]);

        breadth = succBreadth;

        this.layout.takeVertAt(breadth, depth, succDepth - 1, edge);
        edge.path.push([breadth, depth, "vert"]);

        depth = succDepth - 1;

        this.layout.takeHorzAt(breadth, succBreadth, depth, edge);
        edge.path.push([succBreadth, depth, "horz"]);
      }
    }
  }

  // Update the block's DOM node's position.
  setBlockPosition() {
    for (const block of this.blocks) {
      const info = this.layout.getInfoAt(block.breadth, block.depth);
      block.left = info.blockLeft + (info.blockWidth - block.width) / 2;
      block.top = info.blockTop;

      block.node.style.left = block.left + "px";
      block.node.style.top = block.top + "px";
    }
  }

  // Create the SVG node for the edges.
  //
  // Only edges use this SVG node. Blocks don't use SVG.
  createSVGNode() {
    this.svg = document.createElementNS(SVG_NS, "svg");
    this.graphContainer.append(this.svg);
    this.svg.setAttribute("width", `${this.size.x}`);
    this.svg.setAttribute("height", `${this.size.y}`);
  }

  // Create the SVG node for each loopback edge, at the left side of the block.
  // This uses the left side because the class members block will use the
  // ports at the right side.
  //
  //    +-------+
  // +->|       |
  // |  | block |
  // +--|       |
  //    +-------+
  //
  createLoopEdgeNodes() {
    const W = 16, H = 20;

    for (const block of this.blocks) {
      if (!block.loopback) {
        continue;
      }

      const edge = block.loopback;

      let path = "";
      let x = block.left;
      let y = block.top + block.height / 2 + H / 2;
      path += `M ${x} ${y}`;
      path += `C ${x - W} ${y} ${x - W} ${y} ${x - W} ${y - H / 2}`;
      path += `C ${x - W} ${y - H} ${x - W} ${y - H} ${x} ${y - H}`;

      const g = document.createElementNS(SVG_NS, "g");
      g.id = "Gide" + edge.id;
      g.setAttribute("data-symbols", edge.syms.join(","));
      g.classList.add("edge");
      const p = document.createElementNS(SVG_NS, "path");
      p.classList.add("interactive-graph-edge");
      p.setAttribute("d", path);
      g.append(p);

      let headPath = "";
      headPath += `M ${x} ${y - H}`;
      headPath += `L ${x - 8} ${y - H - 3}`;
      headPath += `L ${x - 8} ${y - H + 3}`;
      headPath += `Z`;

      const head = document.createElementNS(SVG_NS, "path");
      head.classList.add("interactive-graph-edge-head");
      head.setAttribute("d", headPath);
      g.append(head);

      edge.node = g;

      this.svg.append(g);
    }
  }

  // Create the SVG node for each non-loopback edge.
  createEdgeNodes() {
    for (const edge of this.edges) {
      if (edge.loopback) {
        continue;
      }

      // (From its end in the upper row, and whether that's its succ's.)
      let straight, upward;
      if (this.rowLayout) {
        straight = edge.layoutPoints;
        upward = edge.upward;
      } else {
        const [x1, y1] = this.layout.getPortPos(edge.pred, edge.predSide,
                                              edge.predDir, edge);
        const [x2, y2] = this.layout.getPortPos(edge.succ, edge.succSide,
                                              edge.succDir, edge);

        let x = x1;
        let y = y1;

        const points = [];
        points.push({ x, y });

        for (const [breadth, depth, target] of edge.path) {
          if (target === "horz") {
            y = this.layout.getHorzAt(breadth, depth, edge);
            points.push({ x, y });
          } else if (target === "vert") {
            x = this.layout.getVertAt(breadth, depth, edge);
            points.push({ x, y });
          }
        }

        x = x2;
        points.push({ x, y });
        y = y2;
        points.push({ x, y });

        straight = straightenPoints(points, edge.pred, edge.succ);
        upward = edge.flipped;
      }
      const start = straight[0];
      const end = straight[straight.length - 1];
      const path = roundedPath(straight);

      const g = document.createElementNS(SVG_NS, "g");
      g.id = "Gide" + edge.id;
      g.setAttribute("data-symbols", edge.syms.join(","));
      g.classList.add("edge");
      const p = document.createElementNS(SVG_NS, "path");
      p.classList.add("interactive-graph-edge");
      p.classList.add("interactive-graph-edge-" + edge.kind);
      p.setAttribute("d", path);
      g.append(p);

      let headOnStart = upward;
      switch (edge.kind) {
        case "inheritance":
        case "composition":
        case "aggregation":
          headOnStart = !headOnStart;
          break;
      }

      const headPos = headOnStart ? start : end;
      // (Its head is on the side of its last part's, ex: under its end, for
      // an edge between blocks in a row, which goes back up to its end.)
      const before = straight.length > 1 ? straight[headOnStart ? 1 : straight.length - 2] : headPos;
      const dy = before.y > headPos.y ? 1 : -1;

      let headPath = "";
      switch (edge.kind) {
        case "default":
        case "inheritance":
        case "implementation":
        case "cross-language":
          // normal
          headPath += `M ${headPos.x} ${headPos.y}`;
          headPath += `L ${headPos.x - 3} ${headPos.y + 8 * dy}`;
          headPath += `L ${headPos.x + 3} ${headPos.y + 8 * dy}`;
          break;

        case "composition":
        case "aggregation":
          // diamond
          headPath += `M ${headPos.x} ${headPos.y}`;
          headPath += `L ${headPos.x - 3} ${headPos.y + 5 * dy}`;
          headPath += `L ${headPos.x} ${headPos.y + 10 * dy}`;
          headPath += `L ${headPos.x + 3} ${headPos.y + 5 * dy}`;
          break;

        case "ipc":
          // vee
          headPath += `M ${headPos.x} ${headPos.y}`;
          headPath += `L ${headPos.x - 3} ${headPos.y + 8 * dy}`;
          headPath += `L ${headPos.x} ${headPos.y + 4 * dy}`;
          headPath += `L ${headPos.x + 3} ${headPos.y + 8 * dy}`;
          break;
      }

      headPath += `Z`;

      const head = document.createElementNS(SVG_NS, "path");
      head.classList.add("interactive-graph-edge-head");
      head.classList.add("interactive-graph-edge-head-" + edge.kind);
      head.setAttribute("d", headPath);
      g.append(head);

      this.svg.append(g);

      edge.node = g;

      const dupe = p.cloneNode(false);
      dupe.classList.add("clicktarget");
      dupe.removeAttribute("stroke-dasharray");
      p.after(dupe);
    }
  }

  // Try to fit the graph to the viewport.
  //
  // At this point, the viewport may still be zero-size.
  // In that case, try again after the resize.
  // See the ResizeObserver in the addEventListeners method.
  initialFitToViewport() {
    if (this.viewportSize.x === 0 ||
        this.viewportSize.y === 0) {
      return;
    }
    this.needInitialFitToViewport = false;
    this.fitToViewport(1);
  }

  // Apply pan and zoom, to make the graph fit to the viewport.
  fitToViewport(maxZoom, useTransition=false) {
    this.zoom = this.clampZoom(Math.min(this.viewportSize.x / this.size.x,
                                        this.viewportSize.y / this.size.y,
                                        maxZoom));
    this.translation.x = (this.viewportSize.x - this.size.x * this.zoom) / 2;
    this.translation.y = (this.viewportSize.y - this.size.y * this.zoom) / 2;

    const clampedT = this.clampTranslation(this.translation, this.zoom);
    this.translation.x = clampedT.x;
    this.translation.y = clampedT.y;

    this.updatePanAndZoom(useTransition);
  }

  // Zoom by given ratio, using cx and cy as the center of the zoom operation.
  // If cx and cy are not provided, treat it as the zoom operation at the
  // center of the viewport.
  zoomBy(ratio, cx = undefined, cy = undefined) {
    const oldZoom = this.zoom;
    this.zoom = this.clampZoom(this.zoom * ratio);

    if (cx === undefined) {
      cx = this.viewportSize.x / 2;
    }
    if (cy === undefined) {
      cy = this.viewportSize.y / 2;
    }
    const dx = this.translation.x - cx;
    const dy = this.translation.y - cy;
    this.translation.x = cx + dx * this.zoom / oldZoom;
    this.translation.y = cy + dy * this.zoom / oldZoom;

    const clampedT = this.clampTranslation(this.translation, this.zoom);
    this.translation.x = clampedT.x;
    this.translation.y = clampedT.y;
  }

  // Translate by given offset.
  translateBy(dx, dy) {
    this.translation.x += dx;
    this.translation.y += dy;

    const clampedT = this.clampTranslation(this.translation, this.zoom);
    this.translation.x = clampedT.x;
    this.translation.y = clampedT.y;
  }

  // Show buttons for zoom operation.
  addButtons() {
    const buttons = document.createElement("div");
    buttons.classList.add("interactive-graph-buttons");

    const zoomIn = document.createElement("button");
    zoomIn.classList.add("interactive-graph-button-zoom-in");
    zoomIn.append("+");
    zoomIn.addEventListener("click", e => {
      e.preventDefault();
      this.zoomBy(1.5);
      this.updatePanAndZoom(true);
    });
    buttons.append(zoomIn);

    const zoomOut = document.createElement("button");
    zoomOut.classList.add("interactive-graph-button-zoom-out");
    zoomOut.append("-");
    zoomOut.addEventListener("click", e => {
      e.preventDefault();
      this.zoomBy(1 / 1.5);
      this.updatePanAndZoom(true);
    });
    buttons.append(zoomOut);

    const fit = document.createElement("button");
    fit.classList.add("interactive-graph-button-fit");
    fit.append("Fit");
    fit.addEventListener("click", e => {
      e.preventDefault();
      this.fitToViewport(1, true);
    });
    buttons.append(fit);

    document.body.append(buttons);
  }

  // The following code is copied from iongraph main.js,
  // with some modifications.

  // Add event listeners for pan and zoom operations.
  addEventListeners() {
    this.viewport.addEventListener("wheel", (e) => {
      e.preventDefault();
      let newZoom = this.zoom;
      if (e.ctrlKey) {
        newZoom = this.clampZoom(this.zoom * Math.pow(ZOOM_SENSITIVITY, -e.deltaY * WHEEL_DELTA_SCALE));
        const zoomDelta = newZoom / this.zoom - 1;
        this.zoom = newZoom;
        const { x: gx, y: gy } = this.viewport.getBoundingClientRect();
        const mouseOffsetX = e.clientX - gx - this.translation.x;
        const mouseOffsetY = e.clientY - gy - this.translation.y;
        this.translation.x -= mouseOffsetX * zoomDelta;
        this.translation.y -= mouseOffsetY * zoomDelta;
      } else {
        this.translation.x -= e.deltaX;
        this.translation.y -= e.deltaY;
      }
      const clampedT = this.clampTranslation(this.translation, newZoom);
      this.translation.x = clampedT.x;
      this.translation.y = clampedT.y;
      this.updatePanAndZoom();
    });

    const pinchState = {
      lastX: 0,
      lastY: 0,
      origR: 0,
      origZoom: 1,
      viewportX: 0,
      viewportY: 0,
    };
    function calculatePinchInfo() {
      let x = 0, y = 0, r = 0;
      for (const info of pointerMap.values()) {
        x += info.clientX;
        y += info.clientY;
      }
      x = x / pointerMap.size;
      y = y / pointerMap.size;
      for (const info of pointerMap.values()) {
        r += ((info.clientX - x) ** 2 +
              (info.clientY - y) ** 2) ** 0.5;
      }
      r = r / pointerMap.size;
      return { x, y, r };
    }
    const pointerMap = new Map();
    this.viewport.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) {
        return;
      }
      if (e.target.closest(".interactive-graph-block")) {
        return;
      }
      if (e.target.closest(".edge")) {
        return;
      }
      e.preventDefault();

      ContextMenu.hide();
      this.viewport.setPointerCapture(e.pointerId);
      pointerMap.set(e.pointerId, {
        clientX: e.clientX,
        clientY: e.clientY,
      });

      if (pointerMap.size >= 2) {
        const pinchInfo = calculatePinchInfo();
        pinchState.lastX = pinchInfo.x;
        pinchState.lastY = pinchInfo.y;
        pinchState.origR = pinchInfo.r;
        pinchState.origZoom = this.zoom;

        const viewportRect = this.viewport.getBoundingClientRect();
        pinchState.viewportX = viewportRect.x;
        pinchState.viewportY = viewportRect.y;
      }
    });
    this.viewport.addEventListener("pointermove", (e) => {
      if (!this.viewport.hasPointerCapture(e.pointerId)) {
        return;
      }

      if (!pointerMap.has(e.pointerId)) {
        return;
      }

      const info = pointerMap.get(e.pointerId);
      const last = {
        clientX: info.clientX,
        clientY: info.clientY,
      };
      info.clientX = e.clientX;
      info.clientY = e.clientY;

      if (pointerMap.size >= 2) {
        const pinchInfo = calculatePinchInfo();

        const scale = pinchInfo.r / pinchState.origR;

        const dx = pinchInfo.x - pinchState.lastX;
        const dy = pinchInfo.y - pinchState.lastY;

        pinchState.lastX = pinchInfo.x;
        pinchState.lastY = pinchInfo.y;

        this.zoomBy(pinchState.origZoom * scale / this.zoom,
                    pinchState.lastX - pinchState.viewportX,
                    pinchState.lastY - pinchState.viewportY);
        this.translateBy(dx, dy);
        this.updatePanAndZoom();
        return;
      }

      const dx = info.clientX - last.clientX;
      const dy = info.clientY - last.clientY;

      this.translateBy(dx, dy);
      this.updatePanAndZoom();
    });
    const onEnd = e => {
      pointerMap.delete(e.pointerId);
      if (!this.viewport.hasPointerCapture(e.pointerId)) {
        return;
      }

      this.viewport.releasePointerCapture(e.pointerId);
    };

    this.viewport.addEventListener("pointerup", onEnd);
    this.viewport.addEventListener("pointercancel", onEnd);
    const ro = new ResizeObserver((entries) => {
      const rect = entries[0].contentRect;
      this.viewportSize.x = rect.width;
      this.viewportSize.y = rect.height;

      if (this.needInitialFitToViewport) {
        this.initialFitToViewport();
      }
    });
    ro.observe(this.viewport);

    this.viewport.addEventListener("dblclick", (e) => {
      this.zoomBy(1.5, e.offsetX, e.offsetY);
      this.updatePanAndZoom(true);
    });
  }

  clampTranslation(t, scale) {
    const minX = TRANSLATION_CLAMP_AMOUNT - this.size.x * scale;
    const maxX = this.viewportSize.x - TRANSLATION_CLAMP_AMOUNT;
    const minY = TRANSLATION_CLAMP_AMOUNT - this.size.y * scale;
    const maxY = this.viewportSize.y - TRANSLATION_CLAMP_AMOUNT;
    const newX = clamp(t.x, minX, maxX);
    const newY = clamp(t.y, minY, maxY);
    return { x: newX, y: newY };
  }
  clampZoom(z) {
    return clamp(z, MIN_ZOOM, MAX_ZOOM);
  }

  updatePanAndZoom(useTransition=false) {
    if (this.appliedZoom === this.zoom &&
        this.appliedTranslation.x === this.translation.x &&
        this.appliedTranslation.y === this.translation.y) {
      return;
    }

    this.appliedZoom = this.zoom;
    this.appliedTranslation.x = this.translation.x;
    this.appliedTranslation.y = this.translation.y;

    if (useTransition) {
      this.graphContainer.style.transition = `transform 0.2s ease-out`;

      this.graphContainer.addEventListener("transitionend", () => {
        this.graphContainer.style.transition = "none";
      }, { once: true });
    }

    this.graphContainer.style.transform = `translate(${Math.round(this.translation.x)}px, ${Math.round(this.translation.y)}px) scale(${this.zoom})`;
  }
}

var GRAPH_EXTRA = [];

window.addEventListener("load", () => {
  // (Pages with other graphs make their own, ex: rev-canvas.js.)
  if (typeof GRAPH_INPUT === "undefined") {
    return;
  }
  // Make the page fit the content area, so that no scrollbar is shown.
  document.documentElement.classList.add("for-interactive-graph");

  let sources = [];
  let targets = [];
  for (const { items } of GRAPH_OPTIONS) {
    for (const item of items) {
      if (item.name === "*syms*") {
        sources = item.sources;
        targets = item.targets;
      }
    }
  }

  // (Grouped in clusters with `?clusters=1`, a prototype; see
  // graph-clusters.js.  In rows, see `RowLayout`, or with `?layout=grid`, in
  // the grid.)
  const params = new URLSearchParams(location.search);
  const clustered = params.get("clusters") && typeof defaultClusterBy == "function";
  const layout = params.get("layout") || "rows";
  // The nodes the navigation panel's facets leave out (see facets.js and
  // query_results/graph_input.liquid), as they change.
  const facetItems = document.querySelector(".facet-items");
  const hiddenSyms = () => new Set(
    [...(facetItems?.querySelectorAll("[data-sym][hidden]") || [])].map(item => item.dataset.sym));
  const options = { hiddenSyms: hiddenSyms(), layout };
  if (clustered) {
    options.clusterBy = defaultClusterBy;
  }
  // (Its facets' links to other queries keep them.)
  for (const link of document.querySelectorAll("a.facet-action")) {
    const url = new URL(link.href);
    for (const param of ["clusters", "layout"]) {
      if (params.get(param)) {
        url.searchParams.set(param, params.get(param));
      }
    }
    link.href = url.href;
  }
  const graph = window.interactiveGraphInstance = new Graph(
    document.querySelector("#interactive-graph-viewport"),
    document.querySelector("#interactive-graph-container"),
    GRAPH_INPUT[0], sources, targets,
    GRAPH_EXTRA, options);
  facetItems?.addEventListener("facetschange", () => graph.setHidden(hiddenSyms()));
}, { once: true });
