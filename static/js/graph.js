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

class PanAndZoom {
  constructor(viewport, container, size) {
    document.documentElement.classList.add("with-pan-and-zoom");

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

    this.size = size;

    this.needInitialFitToViewport = true;

    this.addEventListeners();

    this.initialFitToViewport();

    this.addButtons();
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
        // for GraphRenderer node.
        return;
      }
      if (e.target.closest("g.node")) {
        // for server-side rendered node.
        return;
      }
      if (e.target.closest(".edge")) {
        // for both edges.
        return;
      }
      if (e.target.closest(".diagram-overload")) {
        // overload icons.
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

var Diagram = new (class Diagram {
  LABELS = {
    "Pointer strength": {
      "\u{1f4aa}": {
        // from kind = "strong"
        desc: "Strong pointer",
      },
      "\u{2744}\u{fe0f}": {
        // from kind = "unique"
        desc: "Unique pointer",
      },
      "\u{1f4d3}\u{fe0f}": {
        // from kind = "weak"
        desc: "Weak pointer",
      },
      "\u{1f631}": {
        // from kind = "raw"
        desc: "Raw pointer",
      },
      "&": {
        // from kind = "ref"
        desc: "Reference",
      },
      "\u{1fada}": {
        // from kind = "gcref"
        desc: "GC reference",
      },
      "\u{1f4e6}": {
        // from kind = "contains"
        desc: "Contains",
      },
    },
    "Classes and fields": {
      "\u{269b}\u{fe0f}": {
        // from label = "arc" or label = "atomic"
        desc: "Atomic or Atomic reference counted class",
      },
      "\u{1f517}": {
        // from label = "cc"
        desc: "Cycle-collected class",
      },
      "\u{26d3}\u{fe0f}": {
        // from label = "ccrc"
        desc: "Cycle-collected reference counted class",
      },
      "\u{1f517}\u{270f}\u{fe0f}": {
        // from label = "cc-trace"
        desc: "Field referenced in ::cycleCollection::Trace",
      },
      "\u{1f517}\u{1f50d}": {
        // from label = "cc-traverse"
        desc: "Field referenced in ::cycleCollection::Traverse",
      },
      "\u{26d3}\u{fe0f}\u{200d}\u{1f4a5}": {
        // from label = "cc-unlink"
        desc: "Field referenced in ::cycleCollection::Unlink",
      },
      "\u{1f9ee}": {
        // from label = "rc"
        desc: "Reference counted class",
      },
    },
    "Interfaces and super classes": {
      "nsIIReq": {
        // from elide-and-badge
        desc: "nsIInterfaceRequestor",
      },
      "nsIObs": {
        // from elide-and-badge
        desc: "nsIObserver",
      },
      "nsIRun": {
        // from elide-and-badge
        desc: "nsIRunnable",
      },
      "nsI": {
        // from elide-and-badge
        desc: "nsISupports",
      },
      "nsSupWeak": {
        // from elide-and-badge
        desc: "nsSupportsWeakReference",
      },
      "WC": {
        // from elide-and-badge
        desc: "nsWrapperCache",
      },
    },
  };

  constructor() {
    this.hoveredItems = [];

    this.calculateOverload();

    this.addControl();
    this.addBadgeTooltips();
    this.addOverloadIcons();
    this.addEmptyHint();

    this.makeDiagramHoverEdges();
  }

  toggleOverloadList() {
    const list = document.querySelector("#diagram-limit-warning-overload-list");
    const toggle = document.querySelector("#diagram-limit-warning-overload-list-toggle");
    list.classList.toggle("hidden");
    if (list.classList.contains("hidden")) {
      toggle.textContent = "Show";
    } else {
      toggle.textContent = "Hide";
    }
  }

  canIncreaseDepthLimit() {
    const item = this.getOption("depth");
    if (!item) {
      return false;
    }
    return item.value < item.range[1];
  }

  increaseDepthLimit() {
    const item = this.getOption("depth");
    if (!item) {
      return;
    }
    item.value++;
    this.applyOptions();
  }

  calculateOverload() {
    this.overloadInfoPerSym = null;

    if (typeof GRAPH_OVERLOADS === "undefined") {
      return;
    }

    if (GRAPH_OVERLOADS.length === 0) {
      return;
    }

    this.overloadInfoPerSym = new Map();

    for (const overload of GRAPH_OVERLOADS) {
      if (!overload.sym) {
        continue;
      }
      let overloadInfo;
      if (this.overloadInfoPerSym.has(overload.sym)) {
        overloadInfo = this.overloadInfoPerSym.get(overload.sym);
      } else {
        overloadInfo = {
          depth: [],
          other: [],
          canLift: false,
        };
        this.overloadInfoPerSym.set(overload.sym, overloadInfo);
      }

      if (overload.kind.startsWith("DepthLimit")) {
        overloadInfo.depth.push(overload);
      } else {
        overloadInfo.other.push(overload);
        switch (overload.kind) {
          case "UsesPaths":
          case "UsesLines": {
            const item = this.getOption("path-limit");
            if (item && overload.exist < item.range[1]) {
              overloadInfo.canLift = true;
            }
            break;
          }
          case "NodeLimit": {
            const item = this.getOption("node-limit") ||
                  this.getOption("paths-between-node-limit");
            if (item && overload.exist < item.range[1]) {
              overloadInfo.canLift = true;
            }
            break;
          }
        }
      }
    }
  }

  addControl() {
    this.panel = null;
    this.ignoreNodesItem = null;

    if (typeof GRAPH_OPTIONS == "undefined") {
      return;
    }

    this.panel = document.querySelector("#diagram-panel");

    const optionsPane = document.createElement("div");
    optionsPane.id = "diagram-options-pane";

    for (const { section, items } of GRAPH_OPTIONS) {
      const sectionLabel = document.createElement("h3");
      sectionLabel.append(section);
      optionsPane.append(sectionLabel);
      const sectionBox = document.createElement("div");
      sectionBox.classList.add("diagram-panel-section");

      for (const item of items) {
        if (!item.label) {
          continue;
        }
        const label = document.createElement("label");
        label.id = "diagram-option-label-" + item.name;
        label.setAttribute("for", "diagram-option-" + item.name);
        label.append(item.label);
        sectionBox.append(label);

        if ("choices" in item) {
          const select = document.createElement("select");
          select.id = "diagram-option-" + item.name;
          select.setAttribute("aria-labelledby", "diagram-option-label-" + item.name);
          for (const choice of item.choices) {
            const option = document.createElement("option");
            option.value = choice.value;
            option.append(choice.label);
            select.append(option);
          }
          select.value = item.value;

          select.addEventListener("change", () => {
            item.value = select.value;
          });

          sectionBox.append(select);
        } else if ("range" in item) {
          const box = document.createElement("span");

          const min = document.createElement("span");
          min.classList.add("diagram-panel-range-min");
          min.append(item.range[0]);
          box.append(min);

          const range = document.createElement("input");
          range.id = "diagram-option-range-" + item.name;
          range.classList.add("diagram-panel-range");
          range.type = "range";
          range.min = item.range[0];
          range.max = item.range[1];
          range.value = item.value;
          box.append(range);

          const max = document.createElement("span");
          max.classList.add("diagram-panel-range-max");
          max.append(item.range[1]);
          box.append(max);

          const input = document.createElement("input");
          input.id = "diagram-option-" + item.name;
          input.setAttribute("aria-labelledby", "diagram-option-label-" + item.name);
          input.size = 4;
          input.type = "text";
          input.value = item.value;
          box.append(input);

          input.addEventListener("input", () => {
            item.value = input.value;
            range.value = item.value;
          });
          range.addEventListener("input", () => {
            item.value = range.value;
            input.value = item.value;
          });

          sectionBox.append(box);
        } else if ("type" in item && item.type == "string") {
          const input = document.createElement("input");
          input.id = "diagram-option-" + item.name;
          input.value = item.value;
          input.placeholder = item.placeholder;

          input.addEventListener("input", () => {
            item.value = input.value;
          });

          sectionBox.append(input);

          if (item.name == "ignore-nodes") {
            this.ignoreNodesItem = item;
          }
        } else if ("type" in item && item.type == "bool") {
          const input = document.createElement("input");
          input.id = "diagram-option-" + item.name;
          input.setAttribute("aria-labelledby", "diagram-option-label-" + item.name);
          input.type = "checkbox";
          if (item.value) {
            input.checked = true;
          }

          input.addEventListener("change", () => {
            item.value = input.checked;
          });

          sectionBox.append(input);
        } else {
          const unknown = document.createElement("div");
          unknown.append("(unknown)");
          sectionBox.append(unknown);
        }
      }
      optionsPane.append(sectionBox);
    }

    const apply = document.createElement("button");
    apply.append("Apply");

    apply.addEventListener("click", () => {
      this.applyOptions();
    });

    optionsPane.append(apply);
    this.panel.append(optionsPane);

    const legendPane = document.createElement("div");
    legendPane.id = "diagram-legend-pane";

    const legendTitle = document.createElement("h3");
    legendTitle.append("Legend");
    legendPane.append(legendTitle);

    for (const [sectionLabel, section] of Object.entries(this.LABELS)) {
      const legendTitle = document.createElement("h4");
      legendTitle.append(sectionLabel);
      legendPane.append(legendTitle);

      const legend = document.createElement("div");
      legend.classList.add("diagram-legend");
      for (const [label, item] of Object.entries(section)) {
        const labelBox = document.createElement("span");
        labelBox.append(label);
        if (label.codePointAt(0) > 0x7f) {
          labelBox.style.fontSize = "1.2em";
        }
        legend.append(labelBox);

        const descBox = document.createElement("span");
        descBox.append(item.desc);
        legend.append(descBox);
      }
      legendPane.append(legend);
    }

    this.panel.append(legendPane);
  }

  liftLimitFor(sym) {
    if (!this.overloadInfoPerSym.has(sym)) {
      return;
    }
    const overloadInfo = this.overloadInfoPerSym.get(sym);
    for (const overload of overloadInfo.other) {
      this.liftLimit(overload.kind, overload.exist, /* skipApply = */ true);
    }
    this.applyOptions();
  }

  liftLimit(kind, exists, skipApply=false) {
    switch (kind) {
      case "UsesPaths":
      case "UsesLines": {
        const item = this.getOption("path-limit");
        if (item) {
          this.setOption(item, exists + 100);
        }
        break;
      }
      case "NodeLimit": {
        const item = this.getOption("node-limit") ||
              this.getOption("paths-between-node-limit");
        if (item) {
          this.setOption(item, item.range[1]);
        }
        break;
      }
      case "Overrides":
      case "Subclasses":
      case "FieldMemberUses":
        // Unsupported.
        return;
    }

    if (!skipApply) {
      this.applyOptions();
    }
  }

  getOption(name) {
    for (const { section, items } of GRAPH_OPTIONS) {
      for (const item of items) {
        if (!item.label) {
          continue;
        }
        if (item.name === name) {
          return item;
        }
      }
    }
    return null;
  }

  setOption(item, value) {
    item.value = value;
    if ("range" in item) {
      item.value = Math.max(item.value, item.range[0]);
      item.value = Math.min(item.value, item.range[1]);
    }
    return true;
  }

  applyOptions() {
    let query = Dxr.fields.query.value;

    for (const { section, items } of GRAPH_OPTIONS) {
      for (const item of items) {
        if (!item.label) {
          continue;
        }
        const re = new RegExp(" +" + item.name + ":[^ ]+");
        query = query.replace(re, "");
        if (item.value != item.default) {
          query += " " + item.name + ":" + item.value;
        }
      }
    }

    this.loadQuery(query);
  }

  loadQuery(query) {
    Dxr.fields.query.value = query;
    let url = Dxr.constructURL();
    document.location = url;
  }

  togglePanel() {
    if (!this.panel) {
      return;
    }
    this.panel.classList.toggle("hidden");
  }

  addBadgeTooltips() {
    for (const text of document.querySelectorAll(`svg text[text-decoration="underline"]`)) {
      const label = text.textContent;
      for (const section of Object.values(this.LABELS)) {
        if (label in section) {
          const desc = section[label].desc;

          let tooltip = null;

          text.addEventListener("mouseenter", () => {
            if (tooltip) {
              tooltip.remove();
              tooltip = null;
            }

            const rect = text.getBoundingClientRect();
            const x = rect.left + window.scrollX;
            const y = rect.bottom + window.scrollY;

            tooltip = document.createElement("div");
            tooltip.classList.add("diagram-badge-tooltip");
            tooltip.style.left = (x - 16) + "px";
            tooltip.style.top = (y + 8) + "px";

            const main = document.createElement("div");
            main.classList.add("diagram-badge-tooltip-main");
            main.append(desc);
            tooltip.append(main);

            const arrowBox = document.createElement("div");
            arrowBox.classList.add("diagram-badge-tooltip-arrow-box");

            const arrow = document.createElement("div");
            arrow.classList.add("diagram-badge-tooltip-arrow");
            arrowBox.append(arrow);
            tooltip.append(arrowBox);

            document.body.append(tooltip);
          });
          text.addEventListener("mouseleave", () => {
            if (tooltip) {
              tooltip.remove();
              tooltip = null;
            }
          });
        }
      }
    }
  }

  canIgnoreNode() {
    return this.panel && this.ignoreNodesItem;
  }

  ignoreNode(pretty) {
    if (this.ignoreNodesItem.value != "") {
      this.ignoreNodesItem.value += "," + pretty;
    } else {
      this.ignoreNodesItem.value = pretty;
    }
    this.applyOptions();
  }

  addOverloadIcons() {
    if (!this.overloadInfoPerSym) {
      return;
    }

    for (const graph of document.querySelectorAll("g.graph")) {
      for (const [sym, overloadInfo] of this.overloadInfoPerSym) {
        let isDepthOnly = overloadInfo.other.length === 0;

        const node = graph.querySelector(`[data-symbols*="${sym}"]`);
        if (!node) {
          continue;
        }
        const syms = node.getAttribute("data-symbols").split(",");
        if (!syms.includes(sym)) {
          continue;
        }

        const polygon = node.querySelector("polygon");
        if (!polygon) {
          continue;
        }
        const points = polygon.getAttribute("points");
        if (!points) {
          continue;
        }
        let w = 0, h = 0;
        for (const p of points.split(" ")) {
          const [x, y] = p.split(",").map(n => parseFloat(n));
          w = Math.max(w, x);
          h = Math.min(h, y);
        }
        const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
        g.classList.add("diagram-overload");
        if (isDepthOnly) {
          g.classList.add("diagram-overload-depth");
        } else {
          g.classList.add("diagram-overload-other");
        }
        g.setAttribute("data-diagram-overload-symbol", sym);
        graph.append(g);
        const cx = w - 3;
        const cy = h + 3;
        if (isDepthOnly) {
          const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
          circle.setAttribute("cx", cx);
          circle.setAttribute("cy", cy);
          circle.setAttribute("r", 6);
          g.append(circle);
        } else {
          const triangle = document.createElementNS("http://www.w3.org/2000/svg", "polygon");
          const points = [
            [cx - 6, cy + 5],
            [cx - 6.5, cy + 4],
            [cx - 1, cy - 6],
            [cx + 1, cy - 6],
            [cx + 6.5, cy + 4],
            [cx + 6, cy + 5],
            [cx - 6, cy + 5],
          ];
          triangle.setAttribute("points", points.map(p => p.join(",")).join(" "));
          g.append(triangle);
        }
        const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
        text.setAttribute("x", cx - 3);
        text.setAttribute("y", cy + 3);
        text.setAttribute("font-family", "Courier New");
        text.setAttribute("font-size", "10.0");
        text.setAttribute("font-weight", "bold");
        text.append("!");
        g.append(text);
      }
    }
  }

  getLabelForOverload(overload) {
    if (overload.kind.startsWith("DepthLimit")) {
      return this.overloadToDescription(overload);
    }

    const message = this.overloadToDescription(overload);
    const eq = this.overloadToEq(overload);

    return `${message}: ${eq}: only ${overload.included} included`;
  }

  overloadToDescription(overload) {
    switch (overload.kind) {
      case "Overrides":
        return "Too many overrides";
      case "Subclasses":
        return "Too many subclasses";
      case "UsesPaths":
        return "Too many uses";
      case "UsesLines":
        return "Too many uses lines";
      case "FieldMemberUses":
        return "Too many field member uses";
      case "NodeLimit":
        return "Too many nodes";
      case "DepthLimitOnFieldPointer":
        return "Field pointers are not traversed";
      case "DepthLimitOnBindingSlot":
        return "Binding slots are not traversed";
      case "DepthLimitOnOntologySlot":
        return "Ontology slots are not traversed ";
      case "DepthLimitOnSubclass":
        return "Subclasses are not traversed";
      case "DepthLimitOnSuper":
        return "Super classes are not traversed";
      case "DepthLimitOnOverrides":
        return "Override-edges are not traversed";
      case "DepthLimitOnOverridenBy":
        return "OverriddenBy-edges are not traversed";
      case "DepthLimitOnCallees":
        return "Callees-edges are not traversed";
      case "DepthLimitOnUses":
        return "Uses-edges are not traversed";
      case "DepthLimitOnFieldMemberUses":
        return "Field member uses are not traversed";
      default:
        return `Unknown kind ${overload.kind}`;
    }
  }

  overloadToEq(overload) {
    let limit;
    if (overload.local_limit) {
      limit = overload.local_limit;
    } else {
      limit = overload.global_limit;
    }
    let exist;
    if (overload.exist == 0) {
      exist = "(unknown)";
    } else {
      exist = overload.exist;
    }
    return `${exist} >= ${limit}`;
  }

  toggleOverloadList() {
    const list = document.querySelector("#diagram-limit-warning-overload-list");
    const toggle = document.querySelector("#diagram-limit-warning-overload-list-toggle");
    list.classList.toggle("hidden");
    if (list.classList.contains("hidden")) {
      toggle.textContent = "Show details";
    } else {
      toggle.textContent = "Hide details";
    }
  }

  addEmptyHint() {
    if (!this.panel) {
      return;
    }

    if (Object.keys(SYM_INFO).length > 0) {
      return;
    }

    const query = Dxr.fields.query.value;
    let type;
    if (query.match(/calls-between:.+ calls-between:/)) {
      type = "undirected";
    } else if (query.includes("calls-between-source:") &&
          query.includes("calls-between-target:")) {
      type = "directed";
    } else {
      return;
    }

    const hint = document.createElement("div");

    hint.classList.add("diagram-no-path-hint");
    const header = document.createElement("h3");
    header.append("No path found");
    hint.append(header);

    const p = document.createElement("p");
    p.append("No path found between the specified nodes. This can be caused by the following reasons:");
    hint.append(p);

    const ul = document.createElement("ul");
    if (type === "directed") {
      const li = document.createElement("li");

      li.append("There's a path in the opposite direction. ");

      {
        const button = document.createElement("button");
        button.append("Flip the direction");
        button.addEventListener("click", () => {
          this.flipDirection();
        });
        li.append(button);
      }

      li.append(" ");

      {
        const button = document.createElement("button");
        button.append("Use undirected diagram");
        button.addEventListener("click", () => {
          this.useUndirected();
        });
        li.append(button);
      }

      ul.append(li);
    }
    {
      const li = document.createElement("li");

      li.append("The path is longer than the specified depth. ");

      const button = document.createElement("button");
      button.append("Increase the depth");
      button.addEventListener("click", () => {
        this.increaseDepthLimit();
      });
      li.append(button);

      ul.append(li);
    }
    hint.append(ul);

    this.panel.after(hint);
  }

  flipDirection() {
    let query = Dxr.fields.query.value;
    query = query.replace(/calls-between(-source|-target):/g, m => {
      if (m === "calls-between-source:") {
        return "calls-between-target:";
      }
      return "calls-between-source:";
    });
    this.loadQuery(query);
  }

  useUndirected() {
    let query = Dxr.fields.query.value;
    query = query.replace(/calls-between(-source|-target):/g, m => {
      return "calls-between:";
    });
    this.loadQuery(query);
  }
  
  // In order to provide more useful click/hover targets for diagram edges, we
  // duplicate line body "path" element to create one with a wider stroke that
  // is not visible.
  makeDiagramHoverEdges() {
    const diag = document.querySelector("svg");
    if (!diag) {
      return;
    }

    const edges = diag.querySelectorAll("g.edge > path");
    for (const path of edges) {
      // The default "dotted" style is hard to see.
      // the "dashed" style uses "5,2".
      if (path.getAttribute("stroke-dasharray") === "1,5") {
        path.setAttribute("stroke-dasharray", "2,3");
      }

      const dupe = path.cloneNode(false);
      dupe.classList.add("clicktarget");
      // Dashed/dotted edge should be clickable even in the gap part.
      dupe.removeAttribute("stroke-dasharray");
      // let's insert the clicktarget after the actual path so it is always what
      // the hit test finds.
      path.insertAdjacentElement("afterend", dupe);
    }
  }

  #edgeReverseMap
  // Derive a map from edges to the source and target nodes by processing the
  // GRAPH_EXTRA node data on first use.  This could be generated on the server
  // but since the data is easily derived and we expect our graphs to be
  // O(1000), we don't expect this computation to be too bad.
  #ensureEdgeReverseMap() {
    if (this.#edgeReverseMap) {
      return;
    }

    this.#edgeReverseMap = new Map();
    if (!GRAPH_EXTRA?.[0]) {
      return;
    }

    for (const [node, nodeInfo] of Object.entries(GRAPH_EXTRA[0].nodes)) {
      for (const inEdge of nodeInfo.in_edges) {
        let edgeInfo = this.#edgeReverseMap.get(inEdge);
        if (!edgeInfo) {
          this.#edgeReverseMap.set(inEdge, [undefined, node]);
        } else {
          edgeInfo[1] = node;
        }
      }
      for (const outEdge of nodeInfo.out_edges) {
        let edgeInfo = this.#edgeReverseMap.get(outEdge);
        if (!edgeInfo) {
          this.#edgeReverseMap.set(outEdge, [node, undefined]);
        } else {
          edgeInfo[0] = node;
        }
      }
    }
  }

  maybeActivateHover(elem) {
    if (elem.tagName !== "g" && !elem.classList.contains("interactive-graph-block")) {
      return;
    }

    // We're hovering over a graph so we also want to hover related graph nodes.
    // We will still also potentially want to highlight any document spans as
    // well.
    this.activateHover(elem);
  }

  activateHover(elem) {
    this.deactivateHover();

    let id;
    if (elem.id) {
      id = elem.id;
    } else {
      id = elem.parentElement.id;
    }
    if (id.startsWith("a_")) {
      id = id.substring(2);
    }

    const applyStyling = (targetId, clazzes) => {
      let maybeTarget = document.getElementById(targetId);
      // For the table rows, the id ends up on a "g" container with an "a_"
      // prefix.  We want to locate the a_ prefix and then adjust to its sole
      // child for consistency.
      if (!maybeTarget) {
        maybeTarget = document.getElementById(`a_${targetId}`);
        if (!maybeTarget) {
          return;
        }
        maybeTarget = maybeTarget.children[0];
      }
      maybeTarget.classList.add(...clazzes);

      this.hoveredItems.push([maybeTarget, clazzes]);
    };

    // ## Hovered Edge
    if (id.startsWith("Gide")) {
      const edgeExtra = GRAPH_EXTRA[0].edges[id];
      if (!edgeExtra) {
        return;
      }

      this.#ensureEdgeReverseMap();

      const curEdgeHover = ["hovered-cur-edge"];
      elem.classList.add(...curEdgeHover);
      this.hoveredItems.push([elem, curEdgeHover]);

      let [srcNode, targNode] = this.#edgeReverseMap.get(id);

      const defaultInNodeHover = ["hovered-in-node"];
      applyStyling(srcNode, defaultInNodeHover);

      const defaultOutNodeHover = ["hovered-out-node"];
      applyStyling(targNode, defaultOutNodeHover);

      return;
    }

    let nodeExtra = GRAPH_EXTRA[0].nodes[id];
    if (!nodeExtra) {
      return;
    }

    // ## Hovered Node
    const curNodeHover = ["hovered-cur-node"];
    elem.classList.add(...curNodeHover);
    this.hoveredItems.push([elem, curNodeHover]);

    const defaultInNodeHover = ["hovered-in-node"];
    for (const [nid, clazzes] of nodeExtra.in_nodes) {
      applyStyling(nid, clazzes.length ? clazzes : defaultInNodeHover);
    }
    const defaultOutNodeHover = ["hovered-out-node"];
    for (const [nid, clazzes] of nodeExtra.out_nodes) {
      applyStyling(nid, clazzes.length ? clazzes : defaultOutNodeHover);
    }

    const inEdgeHover = ["hovered-in-edge"];
    for (const eid of nodeExtra.in_edges) {
      applyStyling(eid, inEdgeHover);
    }

    const outEdgeHover = ["hovered-out-edge"];
    for (const eid of nodeExtra.out_edges) {
      applyStyling(eid, outEdgeHover);
    }
  }

  deactivateHover() {
    for (const [item, clazzes] of this.hoveredItems) {
      item.classList.remove(...clazzes);
    }
    this.hoveredItems = [];
  }
})();

// Construct a graph for the given input data.
class GraphRenderer {
  constructor(container, input, sources, targets, extraList) {
    this.graphContainer = container;

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

    this.populateExtra(extraList);
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
}

if (typeof GRAPH_INPUT !== "undefined") {
  window.addEventListener("load", () => {
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

    const viewport = document.querySelector("#interactive-graph-viewport");
    const container = document.querySelector("#interactive-graph-container");

    const graph = new GraphRenderer(
      container, GRAPH_INPUT[0], sources, targets, GRAPH_EXTRA);

    new PanAndZoom(viewport, container, graph.size);
  }, { once: true });
} else if (typeof GRAPH_EXTRA !== "undefined" && GRAPH_EXTRA.length === 1) {
  // The pan/zoom functionalities can be used only when there's only one
  // graph.
  const diag = document.querySelector("svg");
  const rect = diag.getBoundingClientRect();
  const size = { x: rect.width, y: rect.height };

  window.addEventListener("load", () => {
    const viewport = document.querySelector("#interactive-graph-viewport");
    const container = document.querySelector("#interactive-graph-container");

    new PanAndZoom(viewport, container, size);
  }, { once: true });
}
