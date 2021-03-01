/* jshint devel:true, esnext: true */

/**
 * This file consists of four major pieces of functionality for a file view:
 * 0) Any combination of 1) and 2)
 * 1) Multi-select highlight lines with shift key and update window.location.hash
 * 2) Multi-select highlight lines with command/control key and update window.location.hash
 * 3) Highlight lines when page loads, if window.location.hash exists
 */

var Sticky = new (class Sticky {
  constructor() {
    // List of already stuck elements.
    this.stuck = [];
    this.scroller = document.getElementById("scrolling");

    // Our logic can't work on our diff output because there will be line
    // number discontinuities and line numbers that are simply missing.
    let hasLineNumbers = !!document.querySelector(".line-number");
    let isDiffView = document.getElementById("content").classList.contains("diff");
    if (hasLineNumbers && !isDiffView) {
      this.scroller.addEventListener("scroll", () => this.handleScroll(), {
        passive: true,
      });
    }
  }

  /**
   * Hacky but workable sticky detection logic.
   *
   * document.elementsFromPoint can give us the stack of all of the boxes that
   * occur at a given point in the viewport.  The naive assumption that we can
   * look at the stack of returned elements and see if there are two
   * "source-line-with-number" in the stack (one for the sticky bit, one for the
   * actual source it's occluding) turns out to run into problems when sticky
   * things start or stop stickying.  Also, you potentially have to probe twice
   * with a second offset to compensate for exclusive box boundary issues.
   *
   * So instead we can leverage some important facts:
   * - Our sticky lines line up perfectly.  They're always fully visible.
   *   (That said, given that fractional pixel sizes can happen with scaling and
   *   all that, it's likely smart to avoid doing exact math on that.)
   * - We've annotated every line with line numbers.  So any discontinuity greater
   *   than 1 is an indication of a stuck line.  Unfortunately, since we also
   *   expect that sometimes there will be consecutive stuck lines, we can't treat
   *   lack of discontinuity as proof that things aren't stuck.  However, we can
   *   leverage math by making sure that a line beyond our maximum nesting level's
   *   line number lines up.
   */
  handleScroll() {
    const MAX_NESTING = 10;
    const scrolling = document.getElementById("scrolling");
    const firstSourceY = scrolling.offsetTop;
    // The goal is to make sure we're in the area the source line numbers live.
    const lineForSizing = document.querySelector(".line-number");
    const lineRect = lineForSizing.getBoundingClientRect();
    const sourceLinesX = lineRect.left + 6;
    const lineHeight = lineRect.height;

    let firstStuck = null;
    let lastStuck = null;
    const jitter = 6;

    function extractLineNumberFromElem(elem) {
      if (!elem.classList.contains("line-number")) {
        return null;
      }
      let num = parseInt(elem.dataset.lineNumber, 10);
      if (isNaN(num) || num < 0) {
        return null;
      }
      return num;
    }

    /**
     * Walk at a fixed offset into the middle of what should be stuck line
     * numbers.
     *
     * If we don't find a line-number, then we expect that to be due to the
     * transition from stuck elements to partially-scrolled source lines.  It
     * means the current set of lines are all stuck.
     *
     * If we do find a line-number, then we have to look at the actual line
     * number.  If it's consecutive with the previous line, then it means the
     * previous line AND this line are both not stuck, and we should return
     * what we had exclusive of the previous line.
     */
    function walkLinesAndGetLines() {
      let offset = 6;
      let sourceLines = [];

      // Find a line number that can't possibly be stuck.
      let sanityCheckLineNum = extractLineNumberFromElem(
        document.elementFromPoint(
          sourceLinesX,
          firstSourceY + offset + lineHeight * MAX_NESTING
        )
      );
      // if we didn't find a line, try again with a slight jitter because there
      // might have been a box boundary edge-case.
      if (!sanityCheckLineNum) {
        sanityCheckLineNum = extractLineNumberFromElem(
          document.elementFromPoint(
            sourceLinesX,
            jitter + firstSourceY + offset + lineHeight * MAX_NESTING
          )
        );
      }

      // If we couldn't find a sanity-checking line number, then either our logic
      // is broken or the file doesn't have enough lines to necessitate the sticky
      // logic.  Just bail.
      if (!sanityCheckLineNum) {
        return sourceLines;
      }

      for (let iLine = 0; iLine <= MAX_NESTING; iLine++) {
        let elem = document.elementFromPoint(
          sourceLinesX,
          firstSourceY + offset
        );
        if (!elem || !elem.classList.contains("line-number")) {
          break;
        }

        let lineNum = parseInt(elem.dataset.lineNumber, 10);

        let expectedLineNum = sanityCheckLineNum - MAX_NESTING + iLine;
        if (lineNum !== expectedLineNum) {
          // the line-number's parent is the source-line-with-number
          sourceLines.push(elem.parentElement);
        } else {
          break;
        }

        offset += lineHeight;
      }

      return sourceLines;
    }

    let newlyStuckElements = walkLinesAndGetLines();

    let noLongerStuck = new Set(this.stuck);
    for (let elem of newlyStuckElements) {
      elem.classList.add("stuck");
      noLongerStuck.delete(elem);
    }
    let lastElem = null;
    if (newlyStuckElements.length) {
      lastElem = newlyStuckElements[newlyStuckElements.length - 1];
    }
    let prevLastElem = null;
    if (this.stuck.length) {
      prevLastElem = this.stuck[this.stuck.length - 1];
    }
    if (lastElem !== prevLastElem) {
      if (prevLastElem) {
        prevLastElem.classList.remove("last-stuck");
      }
      if (lastElem) {
        lastElem.classList.add("last-stuck");
      }
    }

    for (let elem of noLongerStuck) {
      elem.classList.remove("stuck");
    }

    this.stuck = newlyStuckElements;
  }
})();

var Highlight = new (class Highlight {
  constructor() {
    for (let line of document.querySelectorAll(".line-number")) {
      line.addEventListener("click", event => this.onLineNumberClick(event));
    }
    this.lastSelectedLine = null;
    this.selectedLines = new Set();
    this.updateFromHash();
    window.addEventListener("hashchange", () => {
      this.updateFromHash();
    });
  }

  addSelectedLine(line) {
    document.getElementById("line-" + line).classList.add("highlighted");
    // NOTE: The order here is intentional so that we throw above if the line
    // is not in the document.
    this.selectedLines.add(line);
    this.lastSelectedLine = line;
  }

  removeSelectedLine(line) {
    this.selectedLines.delete(line);
    if (this.lastSelectedLine == line) {
      this.lastSelectedLine = null;
    }
    document.getElementById("line-" + line).classList.remove("highlighted");
  }

  toggleSelectedLine(line) {
    if (this.selectedLines.has(line)) {
      this.removeSelectedLine(line);
    } else {
      this.addSelectedLine(line);
    }
  }

  removeAllLines() {
    for (let line of [...this.selectedLines]) {
      this.removeSelectedLine(line);
    }
  }

  selectSingleLine(line) {
    this.removeAllLines();
    this.addSelectedLine(line);
  }

  onLineNumberClick(event) {
    if (!event.shiftKey && !event.ctrlKey) {
      // Hacky logic to jump if this is a stuck line number
      //
      // TODO(emilio): This should probably select the line as well, or something?
      let containingLine = event.target.closest(".source-line-with-number");
      if (containingLine && containingLine.classList.contains("stuck")) {
        let nestingContainer = containingLine.closest(".nesting-container");
        if (nestingContainer) {
          Sticky.scroller.scrollTop -=
              containingLine.offsetTop - nestingContainer.offsetTop;
        }
        return;
      }
    }

    let line = parseInt(event.target.dataset.lineNumber, 10);
    if (event.shiftKey) {
      // Range-select on shiftkey.
      //
      // TODO(emilio): This should maybe toggle instead of just adding to the
      // selection?
      if (!this.lastSelectedLine) {
        this.addSelectedLine(line);
      } else if (this.lastSelectedLine == line) {
        this.removeSelectedLine(line);
      } else if (this.lastSelectedLine < line) {
        for (let i = this.lastSelectedLine; i < line; ++i) {
          this.addSelectedLine(i + 1);
        }
      } else {
        for (let i = this.lastSelectedLine; i > line; --i) {
          this.addSelectedLine(i - 1);
        }
      }
    } else if (event.ctrlKey || event.metaKey) {
      // Toggle select on ctrl / meta.
      this.toggleSelectedLine(line);
    } else {
      this.selectSingleLine(line);
    }
    this.updateHash();
  }

  /**
   * Creates a synthetic anchor for all hash configurations, even ones that
   * highlight more than one line and therefore can't be understood by the
   * browser's native anchor-seeking like "#200-205" and "#200,205".
   *
   * Even if it seemed like a good idea to attempt to manually trigger this
   * scrolling on load and the "hashchange" event, Firefox notably will manually
   * seek to an anchor if you press the enter key in the location bar and have not
   * changed the hash.  This is a UX flow used by many developers, so it's
   * essential the synthetic anchor is in place.  For this reason, any
   * manipulation of history state via replaceState must call this method.
   *
   * This synthetic anchor also doubles as a means of creating sufficient padding
   * so that "position:sticky" stuck lines don't obscure the line we're seeking
   * to.  (That's what the "goto" class accomplishes.)  Please see mosearch.css
   * for some additional details and context here.
   */
  createSyntheticAnchor(id) {
    if (document.getElementById(id)) {
      return;
    }

    // XXX A bit hackish.
    let firstLineno = id.split(/[,-]/)[0];
    let anchor = document.createElement("div");
    anchor.id = id;
    anchor.className = "goto";

    let elt = document.getElementById("line-" + firstLineno);
    if (elt.classList.contains("nesting-sticky-line")) {
      // As an special-case, if this is a sticky line, we insert the anchor in
      // the container, so that it has its static position.
      elt.parentNode.insertBefore(anchor, elt);
    } else {
      elt.insertBefore(anchor, elt.firstChild);
    }
  }

  updateHash() {
    let hash = this.toHash();
    {
      let historyHash = hash ? "#" + hash : "";
      if (historyHash != window.location.hash) {
        window.history.replaceState(null, "", historyHash);
      }
    }
    if (hash) {
      this.createSyntheticAnchor(hash);
    }
    for (let link of document.querySelectorAll("a[data-update-link]")) {
      let extra = link.getAttribute("data-update-link").replace("{}", hash);
      link.href = link.getAttribute("data-link") + extra;
    }
  }

  toHash() {
    if (!this.selectedLines.size) {
      return "";
    }
    // Try to create ranges out of the lines.
    let lines = [...this.selectedLines].sort((a, b) => a - b);
    let ranges = [];
    let current = { start: lines[0], end: lines[0] };
    for (let i = 1; i < lines.length; ++i) {
      if (lines[i] == current.end + 1) {
        current.end += 1;
      } else {
        ranges.push(current);
        current = { start: lines[i], end: lines[i] };
      }
    }
    ranges.push(current);
    return ranges
      .map(range => {
        if (range.start == range.end) {
          return range.start + "";
        }
        return range.start + "-" + range.end;
      })
      .join(",");
  }

  updateFromHash() {
    this.removeAllLines();
    let hash = window.location.hash.substring(1);
    if (!hash) {
      return;
    }
    for (let chunk of hash.split(",")) {
      if (!chunk.length) {
        continue;
      }
      let range = chunk.split("-");
      if (range.length == 1) {
        let line = parseInt(range[0], 10);
        if (isNaN(line)) {
          continue;
        }
        try {
          this.addSelectedLine(line);
        } catch (ex) {
          // The line may not be in the document.
        }
      } else if (range.length == 2) {
        let first = parseInt(range[0], 10);
        let second = parseInt(range[1], 10);
        if (isNaN(first) || isNaN(second)) {
          continue;
        }
        // Allow inverted ranges in the url, in case they're manually written
        // or what not.
        let start = Math.min(first, second);
        let end = Math.max(first, second);
        for (let i = start; i <= end; ++i) {
          try {
            this.addSelectedLine(i);
          } catch (ex) {
            // The line may not be in the document.
          }
        }
      } else {
        // Something unknown, ignore.
      }
    }
    // We're done parsing the hash, now update so we use the sanitized version
    // if we have at least one line selected. Otherwise it could be an idref or
    // something of that sort.
    if (this.selectedLines.size) {
      this.updateHash();
    }
  }
})();
