"use strict";

// The client-side diagrams' row layout (graph.js's `RowLayout`): each row's
// blocks as wide as they are, placed over their neighbors where they can be,
// with ports over each other's (so edges between them are straight), and an
// edge between rows which aren't adjacent routed around the blocks between
// them.  diagram_ignore::F1's callers: F10 calls F9, which calls F7 and F8;
// F7 calls F6, F6 F5, F5 F3 and F4, F3 F2, and F2 and F4 F1; and F8 calls F2,
// skipping three rows, and F4's call of F1 skips F2's.
const QUERY = "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive";

const graph = () => frame.contentWindow.interactiveGraphInstance;
const name = block => block.pretty.split("::").pop();
const edgeOf = (from, to) => graph().edges.find(edge => name(edge.pred) == from && name(edge.succ) == to);

add_task(async function test_DiagramRows() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", QUERY);
  ok(graph().rowLayout, "Diagrams are laid out in rows");

  for (const [from, to] of [["F10", "F9"], ["F7", "F6"], ["F6", "F5"], ["F4", "F1"]]) {
    const points = edgeOf(from, to).layoutPoints;
    ok(points.length == 2 && points[0].x == points[1].x, `${from}'s call of ${to} is straight`);
  }

  // No edge goes through a block but its ends.
  const inside = (point, block) =>
    point.x > block.left + 1 && point.x < block.left + block.width - 1 &&
    point.y > block.top + 1 && point.y < block.top + block.height - 1;
  for (const edge of graph().edges) {
    const points = edge.layoutPoints;
    const crossed = new Set();
    for (let i = 1; i < points.length; i++) {
      const [a, b] = [points[i - 1], points[i]];
      const steps = Math.ceil(Math.hypot(b.x - a.x, b.y - a.y) / 4);
      for (let step = 0; step <= steps; step++) {
        const point = { x: a.x + (b.x - a.x) * step / steps, y: a.y + (b.y - a.y) * step / steps };
        for (const block of graph().blocks) {
          if (block !== edge.pred && block !== edge.succ && inside(point, block)) {
            crossed.add(name(block));
          }
        }
      }
    }
    is([...crossed].join(", "), "", `${name(edge.pred)}'s call of ${name(edge.succ)} goes around the other blocks`);
  }
  const skip = edgeOf("F8", "F2").layoutPoints;
  ok(skip[0].y < graph().blocks.find(block => name(block) == "F6").top &&
     skip[skip.length - 1].y > graph().blocks.find(block => name(block) == "F3").top,
     "including F8's call of F2, past F6's, F5's and F3's rows");

  // (And the grid, with `?layout=grid`.)
  await TestUtils.loadQuery("tests", QUERY, { layout: "grid" });
  ok(!graph().rowLayout && graph().layout, "?layout=grid lays it out in the grid");
  ok(frame.contentDocument.querySelectorAll("#interactive-graph-container .edge").length >= 11,
     "with its edges");
});
