"use strict";

// A crash set's view's diagrams of what its crashes' threads were busy with
// (see tools/src/crash_sets.rs's `stack_graphs` and `render_crash_set`):
// each connected part of the graph of their stacks' functions, callers over
// callees, laid out by graph.js's `RowLayout` (a row's functions of the same
// class in a record of them, graph-records.js's), is a box of its own, beside
// the others, each function colored by how many of the stacks have it (a
// heat map, on one scale for all of them) and with its number.  As the
// navigation panel's facets filter the crashes (facets.js's "facetschange"),
// the counts are those of the crashes shown, the diagrams without any hide,
// and the biggest of those shown come first.

(function () {
  const data = document.getElementById("crash-set-graphs");
  const box = document.querySelector(".crash-graphs");
  if (!data || !box || typeof drawRecordGraph == "undefined") {
    return;
  }
  const graphs = JSON.parse(data.textContent);
  const rows = [...document.querySelectorAll(".crash-set-crash")];
  const plural = (n, one, many) => `${n} ${n == 1 ? one : many}`;

  // Lays out `graph` in a box of its own, its functions of the same class in
  // a row as records of them (see graph-records.js).
  function draw(graph) {
    const figure = document.createElement("figure");
    figure.className = "crash-graph";
    const caption = document.createElement("figcaption");
    const canvas = document.createElement("div");
    canvas.className = "crash-graph-canvas";
    figure.append(caption, canvas);
    box.append(figure);

    const blocks = [];
    const makeNode = node => {
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
      // (Measured with the widest count it can have.)
      count.textContent = String(node.count);
      blocks.push({ node, element, count });
      return element;
    };
    const scopeOf = node => graphNameLines(node.function)[0] || null;
    const edgeTitle = edge => `${graph.nodes[edge.from].function} → ${graph.nodes[edge.to].function}`;
    const layout = drawRecordGraph(canvas, graph.nodes, graph.edges, { makeNode, scopeOf, edgeTitle });
    return { graph, figure, caption, blocks, edges: layout.edges, cables: layout.cables };
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
        for (const path of edge.paths) {
          path.classList.toggle("crash-graph-edge-none", counts.edges[e] == 0);
        }
      });
      // (A bus dims when all its edges do.)
      for (const cable of diagram.cables) {
        for (const path of cable.paths) {
          path.classList.toggle("crash-graph-edge-none", cable.edges.every(e => counts.edges[e] == 0));
        }
      }
    });
    // (The biggest of those shown first, else as they were.)
    const order = drawn.map((diagram, i) => i)
      .sort((a, b) => counted[b].stacks - counted[a].stacks || a - b);
    box.replaceChildren(...order.map(i => drawn[i].figure));
  }
  paint();
  document.querySelector(".crash-set-crashes .facet-items")?.addEventListener("facetschange", paint);
})();
