/**
 * Clusters for the client-side diagrams (a prototype; see graph.js's
 * `Graph` and its `clusterBy` option): the blocks are grouped by what they
 * belong to (by default their tree, if the graph has several, their
 * subsystem's segments, and their class; see `graphClusterPath`), and the
 * groups are laid out recursively, as ELK's hierarchical layouts are: each
 * cluster's contents (its blocks and its clusters, as blocks of their sizes)
 * with graph.js's own grid layout (in the spirit of iongraph's: predictable
 * rules rather than global optimization), then the cluster as one block in
 * its parent's.
 *
 * - A subsystem's cluster whose contents are loosely coupled (parts with no
 *   edges between them, or only one, a bridge) and which interact with
 *   other clusters is split into those parts, which are laid out where their
 *   own edges put them (an item alone moves up a level, tagged with its
 *   subsystem).  Classes aren't split.
 * - An edge between blocks in different clusters is drawn once between the
 *   clusters (or blocks) that contain them at their nearest common level,
 *   with how many there are; inside the clusters, its ends are stubs on its
 *   blocks.
 * - Some edges don't take part in the layout at all, and are drawn only as
 *   stubs on both ends ("portals", whose tooltips say where they go): those
 *   closing cycles (as iongraph ignores loops' backedges), and those into an
 *   item which many others at its level call (ex: XPCOM's string helpers).
 * - Edges between rows whose ends are only a little apart are drawn
 *   straight (see graph.js's `straightenPoints`, which its unclustered
 *   layout uses too).
 */

// The padding inside a cluster's box, and its header's height.
const CLUSTER_PAD = 12;
const CLUSTER_HEADER = 22;
// The gap between the items with no edges at their level (which are packed
// in rows under the others), and the widest such rows.
const LOOSE_GAP = 16;
const LOOSE_ROW_MIN_WIDTH = 480;
// An item which this many other items at its level have edges into has
// them drawn as portals.
const HUB_IN_DEGREE = 6;
// How long a stub is.
const STUB_LENGTH = 14;
// An item with at least this many leaves at its level which only it has
// edges into (or roots which only have edges into it) has them grouped (see
// `groupFans`).
const FAN_GROUP_MIN = 6;

/**
 * The clusters of a symbol's block, outermost first: its tree, if the graph
 * has symbols of several trees (`multiTree`), the segments of its
 * subsystem ("Core", "DOM", "Workers"; a spec's symbols' is their spec), and
 * its class, by its structured record (see SYM_INFO).
 */
function graphClusterPath(sym, currentTree, multiTree) {
  const info = (typeof SYM_INFO !== "undefined" && SYM_INFO[sym]) || {};
  const meta = info.meta || {};
  const tree = info.tree || currentTree;
  const path = [];
  if (multiTree) {
    path.push({ key: `tree:${tree}`, label: tree, kind: "tree" });
  }
  if (meta.subsystem) {
    let full = "";
    for (const part of meta.subsystem.split("/")) {
      full = full ? `${full}/${part}` : part;
      path.push({ key: `subsystem:${tree}:${full}`, label: part, kind: "subsystem", full });
    }
  }
  if (meta.parentsym?.startsWith("T_")) {
    path.push({
      key: `class:${tree}:${meta.parentsym}`,
      label: meta.parentsym.slice(2).split("::").pop(),
      kind: "class",
      full: meta.parentsym.slice(2),
    });
  }
  return path;
}

/**
 * The default `clusterBy` for a query's diagram: `graphClusterPath` of its
 * blocks' first symbols.
 */
function defaultClusterBy(blocks) {
  const currentTree = document.getElementById("data")?.getAttribute("data-tree") || "";
  const trees = new Set(blocks.map(block => SYM_INFO?.[block.syms[0]]?.tree || currentTree));
  return block => graphClusterPath(block.syms[0], currentTree, trees.size > 1);
}

let nextClusterId = 0;

function newCluster(key, label, kind, parent, full = label) {
  return { id: nextClusterId++, key, label, kind, full, parent, children: [], blocks: [] };
}

const isBlock = item => Array.isArray(item.syms);
// An edge's caller and callee (graph.js's unclustered layout swaps its
// `pred` and `succ` when it draws it upwards, marking it `flipped`).
const fromOf = edge => (edge.flipped ? edge.succ : edge.pred);
const toOf = edge => (edge.flipped ? edge.pred : edge.succ);

/**
 * The cluster tree of `blocks` (each block's cluster is `block.cluster`),
 * from `pathOf(block)`: clusters with only one block and nothing else are
 * dropped (their block tagged with their label), and a subsystem's cluster
 * with only one cluster in it, and no blocks, is merged with it.
 */
function buildClusterTree(blocks, pathOf) {
  const root = newCluster("", "", "root", null);
  const byKey = new Map();
  for (const block of blocks) {
    const path = pathOf(block);
    // (A class's methods may be of several subsystems, by their files: its
    // cluster is where its first block's put it.)
    const last = path[path.length - 1];
    if (last?.kind === "class" && byKey.has(last.key)) {
      byKey.get(last.key).blocks.push(block);
      continue;
    }
    let parent = root;
    for (const step of path) {
      let cluster = byKey.get(step.key);
      if (!cluster) {
        cluster = newCluster(step.key, step.label, step.kind, parent, step.full);
        byKey.set(step.key, cluster);
        parent.children.push(cluster);
      }
      parent = cluster;
    }
    parent.blocks.push(block);
  }
  const prune = cluster => {
    for (const child of cluster.children) {
      prune(child);
    }
    const children = [];
    for (const child of cluster.children) {
      if (child.children.length === 0 && child.blocks.length === 0) {
        continue;
      }
      if (child.kind !== "tree" && child.children.length === 0 && child.blocks.length === 1) {
        const [block] = child.blocks;
        block.tag = child.kind === "class" ? null : child.full;
        cluster.blocks.push(block);
      } else if (child.kind === "subsystem" && child.blocks.length === 0 &&
                 child.children.length === 1 && child.children[0].kind === "subsystem") {
        const [grandchild] = child.children;
        grandchild.label = `${child.label}/${grandchild.label}`;
        grandchild.parent = cluster;
        children.push(grandchild);
      } else {
        children.push(child);
      }
    }
    cluster.children = children;
  };
  prune(root);
  const assign = cluster => {
    for (const block of cluster.blocks) {
      block.cluster = cluster;
    }
    for (const child of cluster.children) {
      child.parent = cluster;
      assign(child);
    }
  };
  assign(root);
  return root;
}

// The item of `cluster` (one of its blocks, or of its child clusters) which
// has `block`, if it's in the cluster.
function itemOf(block, cluster) {
  let c = block.cluster;
  if (c === cluster) {
    return block;
  }
  while (c && c.parent !== cluster) {
    c = c.parent;
  }
  return c || null;
}

// The connected components of `items`, by `adjacency`.
function components(items, adjacency) {
  const seen = new Set();
  const parts = [];
  for (const item of items) {
    if (seen.has(item)) {
      continue;
    }
    const part = [];
    const queue = [item];
    seen.add(item);
    while (queue.length) {
      const current = queue.shift();
      part.push(current);
      for (const next of adjacency.get(current)) {
        if (!seen.has(next)) {
          seen.add(next);
          queue.push(next);
        }
      }
    }
    parts.push(part);
  }
  return parts;
}

// The bridges of the graph of `part` by `adjacency` (Tarjan's): the edges
// whose removal disconnects it.
function bridges(part, adjacency) {
  const index = new Map();
  const low = new Map();
  const found = [];
  let counter = 0;
  const visit = (node, parent) => {
    index.set(node, counter);
    low.set(node, counter);
    counter++;
    for (const next of adjacency.get(node)) {
      if (!index.has(next)) {
        visit(next, node);
        low.set(node, Math.min(low.get(node), low.get(next)));
        if (low.get(next) > index.get(node)) {
          found.push([node, next]);
        }
      } else if (next !== parent) {
        low.set(node, Math.min(low.get(node), index.get(next)));
      }
    }
  };
  for (const node of part) {
    if (!index.has(node)) {
      visit(node, null);
    }
  }
  return found;
}

// `part` split at its most balanced bridge whose sides both have at least
// two items and edges out of the cluster (`external`), and theirs, and so
// on; or `[part]`.
function splitAtBridges(part, adjacency, external) {
  if (part.length < 4) {
    return [part];
  }
  let best = null;
  for (const [a, b] of bridges(part, adjacency)) {
    adjacency.get(a).delete(b);
    adjacency.get(b).delete(a);
    const [side] = components([a], adjacency);
    adjacency.get(a).add(b);
    adjacency.get(b).add(a);
    const sideSet = new Set(side);
    const other = part.filter(item => !sideSet.has(item));
    const balance = Math.min(side.length, other.length);
    if (side.length >= 2 && other.length >= 2 &&
        side.some(item => external.has(item)) && other.some(item => external.has(item)) &&
        (!best || balance > best.balance)) {
      best = { a, b, side, other, balance };
    }
  }
  if (!best) {
    return [part];
  }
  adjacency.get(best.a).delete(best.b);
  adjacency.get(best.b).delete(best.a);
  return [
    ...splitAtBridges(best.side, adjacency, external),
    ...splitAtBridges(best.other, adjacency, external),
  ];
}

/**
 * Split the subsystems' clusters whose contents are loosely coupled (see
 * the module's comment), from the innermost out.
 */
function splitClusters(cluster, edges) {
  for (const child of cluster.children.slice()) {
    splitClusters(child, edges);
  }
  if (cluster.kind !== "subsystem") {
    return;
  }
  const items = [...cluster.blocks, ...cluster.children];
  if (items.length < 2) {
    return;
  }
  const adjacency = new Map(items.map(item => [item, new Set()]));
  const external = new Set();
  for (const edge of edges) {
    if (edge.loopback) {
      continue;
    }
    const a = itemOf(fromOf(edge), cluster);
    const b = itemOf(toOf(edge), cluster);
    if (a && b) {
      if (a !== b) {
        adjacency.get(a).add(b);
        adjacency.get(b).add(a);
      }
    } else if (a || b) {
      external.add(a || b);
    }
  }
  const parts = components(items, adjacency)
    .flatMap(part => splitAtBridges(part, adjacency, external));
  if (parts.length < 2 || !parts.some(part => part.some(item => external.has(item)))) {
    return;
  }
  const parent = cluster.parent;
  const replacements = [];
  parts.forEach((part, n) => {
    if (part.length === 1) {
      const [item] = part;
      if (isBlock(item)) {
        item.tag = item.tag || cluster.full;
        item.cluster = parent;
        parent.blocks.push(item);
      } else {
        item.label = `${cluster.label} › ${item.label}`;
        item.parent = parent;
        replacements.push(item);
      }
      return;
    }
    const piece = newCluster(`${cluster.key}#${n}`, cluster.label, cluster.kind, parent, cluster.full);
    piece.part = [n + 1, parts.length];
    for (const item of part) {
      if (isBlock(item)) {
        item.cluster = piece;
        piece.blocks.push(item);
      } else {
        item.parent = piece;
        piece.children.push(item);
      }
    }
    replacements.push(piece);
  });
  parent.children.splice(parent.children.indexOf(cluster), 1, ...replacements);
}

// The clusters from a block's up to the root's.
function ancestors(block) {
  const chain = [];
  for (let c = block.cluster; c; c = c.parent) {
    chain.push(c);
  }
  return chain;
}

/**
 * The edges of each cluster's level: the graph's edges between items of it
 * (its blocks and child clusters) at their nearest common cluster, grouped
 * by their items (`originals`), as `cluster.levelEdges`.
 */
function levelEdges(root, edges) {
  const all = [];
  const visit = cluster => {
    cluster.levelEdges = [];
    cluster.levelEdgeMap = new Map();
    cluster.children.forEach(visit);
  };
  visit(root);
  for (const edge of edges) {
    if (edge.loopback) {
      continue;
    }
    const predChain = ancestors(fromOf(edge));
    const succChain = new Set(ancestors(toOf(edge)));
    const common = predChain.find(c => succChain.has(c));
    const a = itemOf(fromOf(edge), common);
    const b = itemOf(toOf(edge), common);
    const key = `${isBlock(a) ? "b" + a.id : "c" + a.id}>${isBlock(b) ? "b" + b.id : "c" + b.id}`;
    let lifted = common.levelEdgeMap.get(key);
    if (!lifted) {
      lifted = {
        pred: a, succ: b, loopback: false, kind: edge.kind, originals: [],
        predSide: "", succSide: "", predDir: "", succDir: "",
        flipped: false, path: [], length: 0, portal: false,
      };
      common.levelEdgeMap.set(key, lifted);
      common.levelEdges.push(lifted);
      all.push(lifted);
    }
    lifted.originals.push(edge);
  }
  return all;
}

/**
 * Group the fans of `cluster`'s level (the items whose only edge at the
 * level is from, or to, the same item, ex: a function's many callees, each
 * calling nothing else there, or a function's many callers, which nothing
 * there calls), when there are at least FAN_GROUP_MIN of them, in clusters
 * of their own ("group"), whose contents are packed (they have no edges
 * among them), so that they don't make a row as wide as there are of them;
 * their edges are then one, from the item to the group (or the group to the
 * item), whose ends in it are stubs.
 */
function groupFans(cluster) {
  const items = [...cluster.children, ...cluster.blocks];
  const into = new Map(items.map(item => [item, []]));
  const out = new Map(items.map(item => [item, []]));
  for (const edge of cluster.levelEdges) {
    out.get(edge.pred).push(edge);
    into.get(edge.succ).push(edge);
  }
  const name = item => (isBlock(item) ? item.pretty.split("::").pop() : clusterTitle(item));
  const fans = [];
  for (const hub of items) {
    const leaves = out.get(hub)
      .map(edge => edge.succ)
      .filter(item => item !== hub && out.get(item).length === 0 && into.get(item).length === 1);
    if (leaves.length >= FAN_GROUP_MIN) {
      fans.push({ hub, members: leaves, down: true, label: `${leaves.length} called by ${name(hub)}` });
    }
    const roots = into.get(hub)
      .map(edge => edge.pred)
      .filter(item => item !== hub && into.get(item).length === 0 && out.get(item).length === 1);
    if (roots.length >= FAN_GROUP_MIN) {
      fans.push({ hub, members: roots, down: false, label: `${roots.length} calling ${name(hub)}` });
    }
  }
  for (const { hub, members, down, label } of fans) {
    const group = newCluster(`${cluster.key}>fan:${hub.id}:${down}`, label, "group", cluster);
    const memberSet = new Set(members);
    for (const item of members) {
      if (isBlock(item)) {
        cluster.blocks.splice(cluster.blocks.indexOf(item), 1);
        group.blocks.push(item);
        item.cluster = group;
      } else {
        cluster.children.splice(cluster.children.indexOf(item), 1);
        group.children.push(item);
        item.parent = group;
      }
    }
    group.levelEdges = [];
    group.levelEdgeMap = new Map();
    cluster.children.push(group);
    // (Its members' edges with the hub, as one.)
    const kept = cluster.levelEdges.filter(edge => !memberSet.has(down ? edge.succ : edge.pred));
    const merged = {
      pred: down ? hub : group,
      succ: down ? group : hub,
      loopback: false,
      kind: "default",
      originals: cluster.levelEdges
        .filter(edge => memberSet.has(down ? edge.succ : edge.pred))
        .flatMap(edge => edge.originals),
      predSide: "", succSide: "", predDir: "", succDir: "",
      flipped: false, path: [], length: 0, portal: false,
    };
    cluster.levelEdges = [...kept, merged];
    layoutCluster(group);
  }
}

/**
 * Which of a level's edges are portals (see the module's comment): those
 * closing cycles, by a depth-first walk from the items without incoming
 * edges, and those into an item with at least HUB_IN_DEGREE others' edges.
 */
function markPortals(items, edges) {
  const incoming = new Map(items.map(item => [item, []]));
  const outgoing = new Map(items.map(item => [item, []]));
  for (const edge of edges) {
    outgoing.get(edge.pred).push(edge);
    incoming.get(edge.succ).push(edge);
  }
  for (const [, into] of incoming) {
    if (into.length >= HUB_IN_DEGREE) {
      for (const edge of into) {
        edge.portal = true;
      }
    }
  }
  const state = new Map();
  const walk = item => {
    state.set(item, "active");
    for (const edge of outgoing.get(item)) {
      if (edge.portal) {
        continue;
      }
      const next = state.get(edge.succ);
      if (next === "active") {
        edge.portal = true;
      } else if (!next) {
        walk(edge.succ);
      }
    }
    state.set(item, "done");
  };
  const roots = items.filter(item => incoming.get(item).every(edge => edge.portal));
  for (const item of [...roots, ...items]) {
    if (!state.has(item)) {
      walk(item);
    }
  }
}

/**
 * Lay out `cluster`'s level (its child clusters first, recursively): its
 * items with edges in graph.js's grid layout, those without in rows under
 * them, setting each item's `relLeft` and `relTop` (from its content's
 * origin) and the cluster's `width` and `height` (with its box's padding
 * and header, but the root's).
 */
function layoutCluster(cluster) {
  for (const child of cluster.children) {
    layoutCluster(child);
  }
  if (cluster.kind !== "group") {
    groupFans(cluster);
  }
  const items = [...cluster.children, ...cluster.blocks];
  for (const item of items) {
    // (Its neighbors at this level, for the grid layout's stand-ins.)
    item.levelPreds = [];
    item.levelSuccs = [];
  }
  markPortals(items, cluster.levelEdges);
  const laidOut = cluster.levelEdges.filter(edge => !edge.portal);
  for (const edge of laidOut) {
    edge.pred.levelSuccs.push(edge.succ);
    edge.succ.levelPreds.push(edge.pred);
  }
  const linked = new Set(laidOut.flatMap(edge => [edge.pred, edge.succ]));
  const gridItems = items.filter(item => linked.has(item));
  const loose = items.filter(item => !linked.has(item));

  // (graph.js's layout steps, over a context whose blocks are the items and
  // whose blocks' neighbors are their neighbors at this level.)
  const context = {
    blocks: gridItems.map(item => ({
      item,
      width: item.width,
      height: item.height,
      preds: [],
      succs: [],
      loopback: null,
      depth: -1,
      breadth: -1,
    })),
    edges: [],
    layout: null,
    maxDepth: 0,
    maxBreadth: 0,
    hasBlock: Graph.prototype.hasBlock,
  };
  const proxyOf = new Map(context.blocks.map(proxy => [proxy.item, proxy]));
  for (const proxy of context.blocks) {
    proxy.preds = proxy.item.levelPreds.map(item => proxyOf.get(item));
    proxy.succs = proxy.item.levelSuccs.map(item => proxyOf.get(item));
  }
  context.edges = laidOut.map(edge => {
    edge.gridEdge = {
      lifted: edge,
      pred: proxyOf.get(edge.pred),
      succ: proxyOf.get(edge.succ),
      loopback: false,
      kind: edge.kind,
      predSide: "", succSide: "", predDir: "", succDir: "",
      flipped: false, path: [], length: 0,
    };
    return edge.gridEdge;
  });
  let gridWidth = 0;
  let gridHeight = 0;
  if (context.blocks.length) {
    const methods = Graph.prototype;
    methods.calculateDepth.call(context);
    methods.compressDepth.call(context);
    methods.calculateBreadth.call(context);
    methods.calculateEdgeDirection.call(context);
    methods.calculateDimension.call(context);
    context.layout = new GridLayout(context.maxBreadth, context.maxDepth);
    methods.takePorts.call(context);
    context.layout.sortPorts();
    for (const proxy of context.blocks) {
      context.layout.addBlock(proxy.breadth, proxy.depth, proxy);
    }
    methods.calculateEdgePath.call(context);
    context.layout.calculatePosition();
    for (const proxy of context.blocks) {
      const info = context.layout.getInfoAt(proxy.breadth, proxy.depth);
      proxy.item.relLeft = info.blockLeft + (info.blockWidth - proxy.width) / 2;
      proxy.item.relTop = info.blockTop;
    }
    gridWidth = context.layout.getWidth();
    gridHeight = context.layout.getHeight();
  }
  cluster.context = context;

  // The loose items, in rows under the grid.
  const area = loose.reduce((sum, item) => sum + (item.width + LOOSE_GAP) * (item.height + LOOSE_GAP), 0);
  const rowWidth = Math.max(gridWidth, LOOSE_ROW_MIN_WIDTH, Math.sqrt(area * 2),
                            ...loose.map(item => item.width + 2 * LOOSE_GAP));
  let x = LOOSE_GAP;
  let y = gridHeight + (gridHeight ? 0 : LOOSE_GAP);
  let rowHeight = 0;
  let looseWidth = 0;
  for (const item of loose) {
    if (x > LOOSE_GAP && x + item.width + LOOSE_GAP > rowWidth) {
      x = LOOSE_GAP;
      y += rowHeight + LOOSE_GAP;
      rowHeight = 0;
    }
    item.relLeft = x;
    item.relTop = y;
    x += item.width + LOOSE_GAP;
    rowHeight = Math.max(rowHeight, item.height);
    looseWidth = Math.max(looseWidth, x);
  }
  const contentWidth = Math.max(gridWidth, looseWidth);
  const contentHeight = loose.length ? y + rowHeight + LOOSE_GAP : gridHeight;
  if (cluster.kind === "root") {
    cluster.width = contentWidth;
    cluster.height = contentHeight;
  } else {
    const labelWidth = (clusterTitle(cluster).length * 7) + 2 * CLUSTER_PAD;
    cluster.width = Math.max(contentWidth + 2 * CLUSTER_PAD, labelWidth);
    cluster.height = contentHeight + CLUSTER_HEADER + CLUSTER_PAD;
  }
}

function clusterTitle(cluster) {
  const part = cluster.part ? ` (part ${cluster.part[0]} of ${cluster.part[1]})` : "";
  return `${cluster.label}${part}`;
}

/**
 * Place `cluster` at (`left`, `top`) and its items in it (setting blocks'
 * DOM nodes' positions), recursively.
 */
function placeCluster(cluster, left, top) {
  cluster.left = left;
  cluster.top = top;
  const inset = cluster.kind === "root" ? { x: 0, y: 0 } : { x: CLUSTER_PAD, y: CLUSTER_HEADER };
  cluster.originX = left + inset.x;
  cluster.originY = top + inset.y;
  for (const block of cluster.blocks) {
    block.left = cluster.originX + block.relLeft;
    block.top = cluster.originY + block.relTop;
    block.node.style.left = block.left + "px";
    block.node.style.top = block.top + "px";
  }
  for (const child of cluster.children) {
    placeCluster(child, cluster.originX + child.relLeft, cluster.originY + child.relTop);
  }
}

// The clusters' boxes, behind the blocks, outermost first.
function createClusterNodes(container, root) {
  const nodes = [];
  const visit = (cluster, depth) => {
    if (cluster.kind !== "root") {
      const box = document.createElement("div");
      box.classList.add("interactive-graph-cluster", `interactive-graph-cluster-${cluster.kind}`);
      box.dataset.depth = depth;
      box.style.left = cluster.left + "px";
      box.style.top = cluster.top + "px";
      box.style.width = cluster.width + "px";
      box.style.height = cluster.height + "px";
      const header = document.createElement("div");
      header.classList.add("interactive-graph-cluster-header");
      header.textContent = clusterTitle(cluster);
      header.title = cluster.kind === "tree" ? `The ${cluster.full} tree`
        : cluster.kind === "class" ? `The class ${cluster.full}`
        : cluster.kind === "group" ? `${cluster.label}, and nothing else here`
        : `The subsystem ${cluster.full}`;
      box.append(header);
      nodes.push(box);
    }
    for (const child of cluster.children) {
      visit(child, depth + 1);
    }
  };
  visit(root, 0);
  container.prepend(...nodes);
}

// An SVG element of `tag` with `attrs` and `classes`.
function svgElement(tag, attrs = {}, ...classes) {
  const element = document.createElementNS(SVG_NS, tag);
  for (const [name, value] of Object.entries(attrs)) {
    element.setAttribute(name, value);
  }
  element.classList.add(...classes);
  return element;
}

/**
 * Lay out the graph's blocks in clusters (see the module's comment): what
 * graph.js's `render` does when it has a `clusterBy` option.
 */
Graph.prototype.renderClustered = function () {
  this.graphContainer.replaceChildren();
  this.resetBlockAndEdgeState();
  for (const block of this.blocks) {
    block.cluster = null;
    block.tag = null;
  }
  const root = buildClusterTree(this.blocks, this.options.clusterBy(this.blocks));
  splitClusters(root, this.edges);
  for (const block of this.blocks) {
    this.createBlockNode(block);
  }
  levelEdges(root, this.edges);
  layoutCluster(root);
  placeCluster(root, 0, 0);
  this.size = { x: root.width, y: root.height };
  createClusterNodes(this.graphContainer, root);
  this.createSVGNode();
  this.drawClusterEdges(root);
  this.createLoopEdgeNodes();
  this.clusterRoot = root;
  this.extraList.length = 0;
  this.populateExtra(this.extraList);
};

/**
 * Draw each level's edges, recursively: those laid out along their routes
 * in their level's grid (between blocks, the graph's own edges; between
 * clusters, one for all of the edges between their blocks, wider with more,
 * whose ends in the clusters are stubs on their blocks), and portals as
 * stubs on their blocks.
 */
Graph.prototype.drawClusterEdges = function (root) {
  const pending = [];
  // An edge's group (with the edge's id, for hovering: see `populateExtra`).
  const groups = new Map();
  const groupOf = edge => {
    if (!groups.has(edge)) {
      const g = svgElement("g", { id: `Gide${edge.id}`, "data-symbols": edge.syms.join(",") }, "edge");
      this.svg.append(g);
      groups.set(edge, g);
      edge.node = g;
    }
    return groups.get(edge);
  };
  const stub = (edge, block, side, title) => {
    pending.push({ edge, block, side, title });
  };

  const visit = cluster => {
    const context = cluster.context;
    for (const lifted of cluster.levelEdges) {
      if (lifted.portal) {
        for (const edge of lifted.originals) {
          const [from, to] = [fromOf(edge), toOf(edge)];
          stub(edge, from, "down", `Calls ${to.pretty} (a portal: drawn only at its ends)`);
          stub(edge, to, "up", `Called by ${from.pretty} (a portal: drawn only at its ends)`);
        }
        continue;
      }
      const grid = lifted.gridEdge;
      const offset = point => ({ x: point[0] + cluster.originX, y: point[1] + cluster.originY });
      const start = offset(context.layout.getPortPos(grid.pred, grid.predSide, grid.predDir, grid));
      const end = offset(context.layout.getPortPos(grid.succ, grid.succSide, grid.succDir, grid));
      let points = [start];
      let { x, y } = start;
      for (const [breadth, depth, target] of grid.path) {
        if (target === "horz") {
          y = context.layout.getHorzAt(breadth, depth, grid) + cluster.originY;
        } else {
          x = context.layout.getVertAt(breadth, depth, grid) + cluster.originX;
        }
        points.push({ x, y });
      }
      points.push({ x: end.x, y });
      points.push(end);
      const blockOf = item => (isBlock(item) ? item : { left: item.left, width: item.width });
      points = straightenPoints(points, blockOf(grid.pred.item), blockOf(grid.succ.item));
      const direct = lifted.originals.length === 1 && isBlock(lifted.pred) && isBlock(lifted.succ);
      const g = direct ? groupOf(lifted.originals[0]) : svgElement("g", {}, "edge", "interactive-graph-cluster-edge-group");
      if (!direct) {
        this.svg.append(g);
      }
      const path = svgElement("path", { d: roundedPath(points) }, "interactive-graph-edge", `interactive-graph-edge-${lifted.kind}`);
      if (!direct) {
        path.classList.add("interactive-graph-cluster-edge");
        path.style.strokeWidth = String(1 + Math.log2(lifted.originals.length));
        const title = svgElement("title");
        title.textContent = `${lifted.originals.length} call${lifted.originals.length == 1 ? "" : "s"}: ` +
          lifted.originals.slice(0, 8).map(edge => `${edge.pred.pretty} → ${edge.succ.pretty}`).join("; ") +
          (lifted.originals.length > 8 ? "; …" : "");
        path.append(title);
      }
      g.append(path);
      const headAt = grid.flipped ? points[0] : points[points.length - 1];
      const dy = grid.flipped ? 1 : -1;
      g.append(svgElement("path", {
        d: `M ${headAt.x} ${headAt.y} L ${headAt.x - 3} ${headAt.y + 8 * dy} L ${headAt.x + 3} ${headAt.y + 8 * dy} Z`,
      }, "interactive-graph-edge-head"));
      if (!direct) {
        // (Its ends in its clusters, on their blocks.)
        for (const edge of lifted.originals) {
          const [from, to] = [fromOf(edge), toOf(edge)];
          if (from !== lifted.pred) {
            stub(edge, from, "down", `Calls ${to.pretty}, outside its cluster`);
          }
          if (to !== lifted.succ) {
            stub(edge, to, "up", `Called by ${from.pretty}, outside its cluster`);
          }
        }
      }
    }
    for (const child of cluster.children) {
      visit(child);
    }
  };
  visit(root);

  // (One stub for each block's side, with how many edges it stands for and
  // their ends in its tooltip; each edge's group has its own copy, for
  // hovering.)
  const sides = new Map();
  for (const { edge, block, side, title } of pending) {
    const key = `${block.id}:${side}`;
    if (!sides.has(key)) {
      sides.set(key, { block, side, entries: [] });
    }
    sides.get(key).entries.push({ edge, title });
  }
  for (const { block, side, entries } of sides.values()) {
    const x = block.left + block.width / 2;
    const y0 = side === "down" ? block.top + block.height : block.top;
    const y1 = side === "down" ? y0 + STUB_LENGTH : y0 - STUB_LENGTH;
    const titles = [...new Set(entries.map(entry => entry.title))];
    const tooltip = titles.slice(0, 12).join("\n") + (titles.length > 12 ? `\n… and ${titles.length - 12} more` : "");
    for (const { edge } of entries) {
      const g = groupOf(edge);
      const line = svgElement("path", { d: `M ${x} ${y0} L ${x} ${y1}` }, "interactive-graph-edge", "interactive-graph-stub");
      const tip = svgElement("circle", { cx: x, cy: y1, r: 2.5 }, "interactive-graph-stub-end");
      const titleNode = svgElement("title");
      titleNode.textContent = tooltip;
      tip.append(titleNode);
      g.append(line, tip);
    }
    if (entries.length > 1) {
      const count = svgElement("text", {
        x: x + 5,
        y: side === "down" ? y1 + 4 : y1 - 1,
      }, "interactive-graph-stub-count");
      count.textContent = String(entries.length);
      const titleNode = svgElement("title");
      titleNode.textContent = tooltip;
      count.append(titleNode);
      this.svg.append(count);
    }
  }
  // (Every edge has a group, for hovering.)
  for (const edge of this.edges) {
    if (!edge.loopback) {
      groupOf(edge);
    }
  }
};
