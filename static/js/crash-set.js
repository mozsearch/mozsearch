"use strict";

// A crash set's view's diagrams of what its crashes' threads were busy with
// (see tools/src/crash_sets.rs's `stack_graphs` and `render_crash_set`):
// each connected part of the graph of their stacks' functions, callers over
// callees, laid out by graph.js's `RowLayout`, is a box of its own, beside
// the others, each function colored by how many of the stacks have it (a
// heat map, on one scale for all of them) and with its number.  As the
// navigation panel's facets filter the crashes (facets.js's "facetschange"),
// the counts are those of the crashes shown, the diagrams without any hide,
// and the biggest of those shown come first.

(function () {
  const data = document.getElementById("crash-set-graphs");
  const box = document.querySelector(".crash-graphs");
  if (!data || !box || typeof RowLayout == "undefined") {
    return;
  }
  const NS = "http://www.w3.org/2000/svg";
  const graphs = JSON.parse(data.textContent);
  const rows = [...document.querySelectorAll(".crash-set-crash")];
  const plural = (n, one, many) => `${n} ${n == 1 ? one : many}`;

  // Lays out `graph` in a box of its own.
  function draw(graph) {
    const figure = document.createElement("figure");
    figure.className = "crash-graph";
    const caption = document.createElement("figcaption");
    const canvas = document.createElement("div");
    canvas.className = "crash-graph-canvas";
    figure.append(caption, canvas);
    box.append(figure);

    const blocks = graph.nodes.map(node => {
      const element = document.createElement("div");
      element.className = "crash-graph-node";
      const [scope, text] = graphNameLines(node.function);
      if (scope) {
        const line = document.createElement("div");
        line.className = "crash-graph-node-scope";
        line.textContent = scope;
        element.append(line);
      }
      const line = document.createElement("div");
      line.className = "crash-graph-node-text";
      const count = document.createElement("span");
      count.className = "crash-graph-node-count";
      line.append(text, " ", count);
      element.append(line);
      canvas.append(element);
      // (Measured with the widest count it can have.)
      count.textContent = String(node.count);
      return { node, element, count, width: element.offsetWidth, height: element.offsetHeight };
    });
    const edges = graph.edges.map(edge => ({ pred: blocks[edge.from], succ: blocks[edge.to] }));
    const size = new RowLayout(blocks, edges, { margin: 6 }).run();
    canvas.style.width = `${size.x}px`;
    canvas.style.height = `${size.y}px`;
    for (const block of blocks) {
      block.element.style.left = `${block.layoutLeft}px`;
      block.element.style.top = `${block.layoutTop}px`;
    }

    const svg = document.createElementNS(NS, "svg");
    svg.classList.add("crash-graph-edges");
    svg.setAttribute("width", size.x);
    svg.setAttribute("height", size.y);
    for (const edge of edges) {
      const points = edge.layoutPoints;
      if (!points || points.length < 2) {
        continue;
      }
      const path = document.createElementNS(NS, "path");
      path.classList.add("crash-graph-edge");
      path.setAttribute("d", roundedPath(points));
      // (Its head at its callee, its succ.)
      const end = edge.upward ? points[0] : points[points.length - 1];
      const before = edge.upward ? points[1] : points[points.length - 2];
      const dy = before.y > end.y ? 1 : -1;
      const head = document.createElementNS(NS, "path");
      head.classList.add("crash-graph-edge-head");
      head.setAttribute("d", `M ${end.x} ${end.y} L ${end.x - 3} ${end.y + 7 * dy} L ${end.x + 3} ${end.y + 7 * dy} Z`);
      svg.append(path, head);
      edge.paths = [path, head];
    }
    canvas.prepend(svg);
    return { graph, figure, caption, blocks, edges };
  }

  const drawn = graphs.map(draw);

  // The counts of the stacks of the crashes shown, the heat on one scale for
  // all the diagrams.
  function paint() {
    const shown = new Set(rows.filter(row => !row.hidden).map(row => row.dataset.path));
    const counted = drawn.map(({ graph }) => {
      const nodes = new Array(graph.nodes.length).fill(0);
      const edges = new Array(graph.edges.length).fill(0);
      const crashes = new Set();
      let stacks = 0;
      for (const stack of graph.stacks) {
        if (!shown.has(stack.crash)) {
          continue;
        }
        stacks++;
        crashes.add(stack.crash);
        stack.nodes.forEach(node => nodes[node]++);
        stack.edges.forEach(edge => edges[edge]++);
      }
      return { nodes, edges, stacks, crashes: crashes.size };
    });
    const max = Math.max(1, ...counted.flatMap(counts => counts.nodes));
    drawn.forEach((diagram, i) => {
      const counts = counted[i];
      diagram.figure.hidden = counts.stacks == 0;
      diagram.caption.textContent =
        `${plural(counts.stacks, "stack", "stacks")} of ${plural(counts.crashes, "crash", "crashes")}`;
      diagram.blocks.forEach((block, n) => {
        const count = counts.nodes[n];
        block.element.style.setProperty("--heat", (count / max).toFixed(3));
        block.element.classList.toggle("crash-graph-node-none", count == 0);
        block.count.textContent = count;
        block.element.title =
          `${block.node.function}\n${block.node.place}\nIn ${count} of the ${plural(counts.stacks, "stack", "stacks")} shown`;
      });
      diagram.edges.forEach((edge, e) => {
        for (const path of edge.paths || []) {
          path.classList.toggle("crash-graph-edge-none", counts.edges[e] == 0);
        }
      });
    });
    // (The biggest of those shown first, else as they were.)
    const order = drawn.map((diagram, i) => i)
      .sort((a, b) => counted[b].stacks - counted[a].stacks || a - b);
    box.replaceChildren(...order.map(i => drawn[i].figure));
  }
  paint();
  document.querySelector(".crash-set-crashes .facet-items")?.addEventListener("facetschange", paint);
})();
