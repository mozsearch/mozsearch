"use strict";

// js-analyze's analyses of scripts as their documents have them: an SVG
// document's script's text, XML's (its text children's, without its
// entities' other files' text), and a Node script, whose top level can
// return.  (Their syntax errors would warn otherwise, which fails the
// webtests' indexing, as would html/elided-handler.html's elided handlers'.)

async function records(path) {
  const response = await fetch(`/tests/raw-analysis/${path}`, { cache: "no-store" });
  is(response.status, 200, `${path} has an analysis`);
  return (await response.text()).split("\n").filter(Boolean).map(line => JSON.parse(line));
}

function def(records, name) {
  return records.find(record => record.target && record.kind == "def" && record.pretty == name);
}

add_task(async function test_SvgScriptText() {
  const svg = await records("html/xml-script-text.svg");
  is(def(svg, "svgScriptBeforeChild")?.loc, "10:14-34", "A script with an element in it is analyzed");
  is(def(svg, "svgScriptAfterChild")?.loc, "10:60-79", "and its text after the element keeps its position");
});

add_task(async function test_NodeScriptReturn() {
  const js = await records("js/node-script.js");
  is(def(js, "nodeScriptArgs")?.loc, "3:6-20", "A Node script with a top-level return is analyzed");
  is(def(js, "nodeScriptMain")?.loc, "8:9-23", "including what follows the return");
});
