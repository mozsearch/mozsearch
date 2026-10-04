/**
 * Interdiff pages (see `format_interdiff` in format.rs): B's diff, whose rows'
 * code has a `data-idiff` attribute naming its tokens' marks ("START:END:MARK"
 * for each token, separated by ";", where START and END are UTF-16 offsets in
 * the code, and MARK is how the token compares with A's version of the patch;
 * see `hyperblame::interdiff`).  We wrap the tokens in `idiff-MARK` spans.
 */
(function () {
  const TITLES = {
    "new": "New in B",
    "same": "The same as A's",
    "kept": "A removed this, but B keeps it",
    "base": "From neither patch (ex: a commit in between)",
    "rm-new": "Only B removed this",
    "rm-same": "A removed this too",
  };

  function textNodes(root) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const nodes = [];
    while (walker.nextNode()) {
      nodes.push(walker.currentNode);
    }
    return nodes;
  }

  /**
   * Wrap the text of `code` in each of the (sorted) ranges in a span, splitting
   * text nodes at the ranges' boundaries (like `TokenBlamePopup.renderReplica`
   * in hyperblame.js, since the code is syntax highlighted).
   */
  function wrapRanges(code, ranges) {
    const boundaries = new Set(ranges.flatMap(r => [r.start, r.end]));
    let offset = 0;
    for (const node of textNodes(code)) {
      const end = offset + node.length;
      for (const boundary of [...boundaries].filter(b => b > offset && b < end).sort((a, b) => b - a)) {
        node.splitText(boundary - offset);
      }
      offset = end;
    }
    offset = 0;
    let r = 0;
    for (const node of textNodes(code)) {
      const start = offset;
      offset += node.length;
      while (r < ranges.length && ranges[r].end <= start) {
        r++;
      }
      const range = ranges[r];
      if (range && range.start <= start && offset <= range.end) {
        const span = document.createElement("span");
        span.className = `idiff-${range.mark}`;
        span.title = TITLES[range.mark] || "";
        node.replaceWith(span);
        span.append(node);
      }
    }
  }

  for (const code of document.querySelectorAll("code[data-idiff]")) {
    const ranges = code.dataset.idiff.split(";").map(attr => {
      const [start, end, mark] = attr.split(":");
      return { start: parseInt(start, 10), end: parseInt(end, 10), mark };
    });
    ranges.sort((a, b) => a.start - b.start);
    wrapRanges(code, ranges);
  }
})();
