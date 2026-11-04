"use strict";

// The lock-held: view's diagrams of how its regions are called (see
// tools/src/lock_held.rs's `region_graphs` and `render_lock_held`): each
// connected part of the graph of the regions holding the mutex and the
// functions they call with it held (and pass its proof to), callers over
// callees, with their unlocks and the other locks they acquire, laid out by
// graph.js's `RowLayout`, is a box of its own, as crash-set.js's are, a row's
// functions of the same class in a record of them (graph-records.js's).  Each
// node's color says how many of the cards shown it stands for (a heat map,
// on one scale for all of them); as the navigation panel's facets filter the
// cards (facets.js's "facetschange"), the nodes without any dim, the edges
// between them too, the diagrams without any hide, and the biggest of those
// shown come first.  Clicking a node goes to its first card shown.

(function () {
  const data = document.getElementById("lock-held-graphs");
  const box = document.querySelector(".lock-held-graphs");
  if (!data || !box || typeof drawRecordGraph == "undefined") {
    return;
  }
  const graphs = JSON.parse(data.textContent);
  const card = i => document.getElementById(`lock-held-card-${i}`);
  const plural = (n, one, many) => `${n} ${n == 1 ? one : many}`;

  // Lays out `graph` in a box of its own, its functions of the same class in
  // a row as records of them (see graph-records.js).
  function draw(graph) {
    const figure = document.createElement("figure");
    figure.className = "crash-graph lock-held-graph";
    const caption = document.createElement("figcaption");
    const canvas = document.createElement("div");
    canvas.className = "crash-graph-canvas";
    figure.append(caption, canvas);
    box.append(figure);

    const blocks = [];
    const makeNode = node => {
      const element = document.createElement("a");
      element.className = `crash-graph-node lock-held-node lock-held-node-${node.kind}`;
      element.href = node.link;
      // (A function's name, by its scope and its last part; a mutex's, as
      // it is.)
      const [scope, text] = node.kind == "lock" || node.kind == "release"
        ? ["", node.label] : graphNameLines(node.label);
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
      const sub = document.createElement("div");
      sub.className = "lock-held-node-sub";
      sub.textContent = node.sub;
      element.append(sub);
      // (Measured with the widest count it can have.)
      count.textContent = String(node.cards.length);
      // (Its first card shown, rather than its source, unless none is.)
      element.addEventListener("click", event => {
        const target = node.cards.map(card).find(each => each && !each.hidden);
        if (target && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
          event.preventDefault();
          target.scrollIntoView({ block: "start", behavior: "smooth" });
        }
      });
      blocks.push({ node, element, count });
      return element;
    };
    // (Functions called with it held, by their classes: not its guards'
    // regions, its unlocks, or the other locks.)
    const scopeOf = node => (["callee", "proof", "contract"].includes(node.kind)
      ? graphNameLines(node.label)[0] || null : null);
    const edgeTitle = edge => `Line${edge.lines.length == 1 ? "" : "s"} ${edge.lines.join(", ")}`;
    const layout = drawRecordGraph(canvas, graph.nodes, graph.edges, { makeNode, scopeOf, edgeTitle });
    return { graph, figure, caption, blocks, edges: layout.edges, cables: layout.cables };
  }

  const drawn = graphs.map(draw);

  // The counts of the cards shown each node stands for, the heat on one
  // scale for all the diagrams.
  function paint() {
    const shown = i => {
      const element = card(i);
      return element && !element.hidden;
    };
    const counted = drawn.map(({ graph }) => graph.nodes.map(node => node.cards.filter(shown).length));
    const max = Math.max(1, ...counted.flat());
    drawn.forEach((diagram, i) => {
      const counts = counted[i];
      const cards = new Set(diagram.graph.nodes.flatMap(node => node.cards).filter(shown));
      diagram.figure.hidden = cards.size == 0;
      diagram.caption.textContent = plural(cards.size, "card", "cards");
      diagram.blocks.forEach((block, n) => {
        const count = counts[n];
        block.element.style.setProperty("--heat", (count / max).toFixed(3));
        block.element.classList.toggle("crash-graph-node-none", count == 0);
        block.count.textContent = count;
        block.element.title = `${block.node.label}\n${block.node.sub}\n${plural(count, "card", "cards")} shown`;
      });
      // (An edge without cards shown at either end dims, and a bus all of
      // whose edges do.)
      const none = diagram.graph.edges.map(edge => counts[edge.from] == 0 || counts[edge.to] == 0);
      diagram.edges.forEach((edge, e) => {
        for (const path of edge.paths) {
          path.classList.toggle("crash-graph-edge-none", none[e]);
        }
      });
      for (const cable of diagram.cables) {
        for (const path of cable.paths) {
          path.classList.toggle("crash-graph-edge-none", cable.edges.every(e => none[e]));
        }
      }
    });
    // (The biggest of those shown first, else as they were.)
    const size = i => new Set(drawn[i].graph.nodes.flatMap(node => node.cards).filter(shown)).size;
    const order = drawn.map((diagram, i) => i).sort((a, b) => size(b) - size(a) || a - b);
    box.replaceChildren(...order.map(i => drawn[i].figure));
  }
  paint();
  document.querySelector(".lock-held-cards")?.addEventListener("facetschange", paint);
})();
