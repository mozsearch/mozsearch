// A vendored build tool's code citing the example Bikeshed spec, as
// firefox-disco's third_party/node/node_modules/webpack's HTML parser cites
// the HTML Standard: its algorithms' views leave it out (the tests tree's
// `excluded_citation_paths`; see test_RenderedSpecs.js), but searches have it.

// https://example.github.io/bs/#poke
function poke(widget) {
  // Step 1.
  widget.state = "poked";
}

module.exports = { poke };
