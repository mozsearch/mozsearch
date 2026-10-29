"use strict";

// The client-side diagrams' clusters (a prototype, with `?clusters=1`; see
// graph-clusters.js): rollup::Engine::Run (in the Core/Rollup Engine
// subsystem) calls rollup::Storage's Put and Get (in Core/Rollup Storage),
// so the diagram has the Core subsystem's box, with Rollup Storage's in it,
// with the Storage class's in that, around Put and Get; Run is alone of its
// subsystem, so it's tagged with it instead; and its calls are one edge from
// Run to Rollup Storage's box, whose ends in it are stubs on Put and Get.
add_task(async function test_DiagramClusters() {
  await TestUtils.resetFeatureGate("diagramming");
  const QUERY = "calls-from:'rollup::Engine::Run' depth:2 graph-format:mozsearch-interactive";

  await TestUtils.loadQuery("tests", QUERY);
  ok(frame.contentDocument.querySelector(".interactive-graph-block"), "The diagram is there");
  ok(!frame.contentDocument.querySelector(".interactive-graph-cluster"),
     "without clusters, at first");

  await TestUtils.loadQuery("tests", QUERY, { clusters: "1" });
  const doc = frame.contentDocument;
  const header = box => box.querySelector(".interactive-graph-cluster-header").textContent;
  const boxes = [...doc.querySelectorAll(".interactive-graph-cluster")];
  is(boxes.map(header).join(", "), "Core, Rollup Storage, Storage",
     "With clusters, the subsystems' and classes' boxes nest");
  const block = name => [...doc.querySelectorAll(".interactive-graph-block")]
    .find(node => node.querySelector(".interactive-graph-block-text")?.textContent == name);
  const inside = (node, box) => {
    const [a, b] = [node.getBoundingClientRect(), box.getBoundingClientRect()];
    return a.left >= b.left && a.right <= b.right && a.top >= b.top && a.bottom <= b.bottom;
  };
  const storage = boxes.find(box => header(box) == "Storage");
  ok(inside(block("Put"), storage) && inside(block("Get"), storage),
     "Put and Get are in the Storage class's box");
  ok(!inside(block("Run"), boxes.find(box => header(box) == "Rollup Storage")) &&
     inside(block("Run"), boxes.find(box => header(box) == "Core")),
     "and Run isn't in Rollup Storage's, but in Core's");
  is(block("Run").querySelector(".interactive-graph-block-tag")?.textContent, "Core/Rollup Engine",
     "tagged with its subsystem, which has only it");
  const edge = doc.querySelector(".interactive-graph-cluster-edge");
  ok(edge?.querySelector("title")?.textContent.startsWith("2 calls"),
     "Its calls are one edge to Rollup Storage's box");
  is(doc.querySelectorAll(".interactive-graph-stub").length, 2,
     "whose ends in it are stubs on Put and Get");
});
