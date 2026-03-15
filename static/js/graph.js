const SVG_NS = "http://www.w3.org/2000/svg";
const ZOOM_SENSITIVITY = 1.5;
const WHEEL_DELTA_SCALE = 0.01;
const MAX_ZOOM = 2;
const MIN_ZOOM = 0.01;
const TRANSLATION_CLAMP_AMOUNT = 0;
const CROSS_MARGIN = 20;
const PORT_MARGIN = 10;
const CROSS_GAP = 8;

function clamp(x, min, max) {
  return Math.max(min, Math.min(max, x));
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

// Construct a graph for the given input data.
class Graph {
  constructor(viewport, container, input, sources, targets, extraList) {
    this.viewport = viewport;
    this.graphContainer = container;

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

    this.createBlocksAndEdges(input, sources, targets);

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

    this.createSVGNode();

    this.createEdgeNodes();

    this.createLoopEdgeNodes();

    this.needInitialFitToViewport = true;

    this.populateExtra(extraList);

    this.addEventListeners();

    this.initialFitToViewport();

    this.addButtons();
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
    for (const sym of input.nodes) {
      const pretty = SYM_INFO[sym].pretty;
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
    const roots = this.blocks.filter(b => b.preds.length === 0);
    for (const block of roots) {
      block.depth = 0;
    }

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
  }

  // calculateDepth algorithm can create a depth with no blocks.
  // Shift those items up.
  compressDepth() {
    let remaining = this.blocks.slice();
    let destDepth = 0;
    let srcDepth = 0;

    while (remaining.length > 0) {
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
        blocksInDepth.sort((a, b) => a.tmpBreadth > b.tmpBreadth);
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

  // Create the DOM node for each block.
  createBlockNodes() {
    for (const block of this.blocks) {
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
      this.graphContainer.append(node);

      const rect = node.getBoundingClientRect();
      block.width = rect.width;
      block.height = rect.height;

      block.node = node;
      this.layout.addBlock(block.breadth, block.depth, block);
    }
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
    this.size = { x: this.layout.getWidth(), y: this.layout.getHeight() };
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

      const R = 64;
      const T = 0.6;

      let path = "";

      const start = points[0];
      path += `M ${start.x} ${start.y} `;

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
          c3y = Math.min(curr.y + R, (curr.y + next.y ) / 2);
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

      const g = document.createElementNS(SVG_NS, "g");
      g.id = "Gide" + edge.id;
      g.setAttribute("data-symbols", edge.syms.join(","));
      g.classList.add("edge");
      const p = document.createElementNS(SVG_NS, "path");
      p.classList.add("interactive-graph-edge");
      p.classList.add("interactive-graph-edge-" + edge.kind);
      p.setAttribute("d", path);
      g.append(p);

      let headOnStart = edge.flipped;
      switch (edge.kind) {
        case "inheritance":
        case "composition":
        case "aggregation":
          headOnStart = !headOnStart;
          break;
      }

      const headPos = headOnStart ? start : end;
      const dy = headOnStart ? 1 : -1;

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

  new Graph(
    document.querySelector("#interactive-graph-viewport"),
    document.querySelector("#interactive-graph-container"),
    GRAPH_INPUT[0], sources, targets,
    GRAPH_EXTRA);
}, { once: true });
