var Panel = new (class Panel {
  constructor() {
    this.panel = document.getElementById("panel");
    // Avoid complaining if this page doesn't have a panel on it.
    if (!this.panel) {
      return;
    }
    this.toggleButton = document.getElementById("panel-toggle");
    this.icon = this.panel.querySelector(".navpanel-toggle-icon");
    this.settingsButton = document.getElementById("show-settings");
    this.content = document.getElementById("panel-content");
    this.accelEnabledCheckbox = document.getElementById("panel-accel-enable");

    this.permalinkNode = document.querySelector("#panel-permalink");
    this.unpermalinkNode = document.querySelector("#panel-remove-permalink");

    this.selectedSymbol = null;

    this.markdown = {
      filename: {
        node: document.querySelector("#panel-copy-filename-link"),
        isEnabled: () => {
          return true;
        },
        getText: url => {
          const filename = new URL(url).pathname.match(/\/([^\/]+)$/)[1];
          return `[${filename}](${url})`;
        },
      },
      symbol: {
        node: document.querySelector("#panel-copy-symbol-link"),
        isEnabled: () => {
          return this.selectedSymbol;
        },
        getText: url => {
          return `[${this.selectedSymbol}](${url})`;
        },
      },
      block: {
        node: document.querySelector("#panel-copy-code-block"),
        isEnabled: () => {
          return Highlighter?.selectedLines.size > 0;
        },
        getText: url => {
          const file = document.getElementById("file");
          const lang = file.getAttribute("data-markdown-slug") || "";
          return [url, "```" + lang, ...this.formatSelectedLines(), "```"].join(
            "\n"
          );
        },
      },
    };

    // We want the default event for clicking on the settings link to work, but
    // we want to stop its propagation so that it doesn't bubble up to the
    // toggle button handler that comes next.
    this.settingsButton.addEventListener("click", (evt) => { evt.stopPropagation(); });

    this.toggleButton.addEventListener("click", () => this.toggle());
    this.accelEnabledCheckbox.addEventListener("change", () => {
      localStorage.setItem("accel-enable", event.target.checked ? "1" : "0");
      this.updateAccelerators();
    });
    document.documentElement.addEventListener("keypress", event =>
      this.maybeHandleAccelerator(event)
    );

    for (let copy of this.panel.querySelectorAll("button.copy")) {
      copy.addEventListener("click", e => {
        e.preventDefault();

        if (copy.hasAttribute("data-copying")) {
          return;
        }

        this.copyText(copy, copy.parentNode.href);
      });
    }

    const updateUnpermalink = () => {
      if (/^\/[^\/]+\/rev\//.test(document.location.pathname)) {
        if (this.unpermalinkNode) {
           this.unpermalinkNode.classList.remove("disabled");
        }
      } else {
        if (this.unpermalinkNode) {
           this.unpermalinkNode.classList.add("disabled");
        }
      }
    };

    updateUnpermalink();

    if (this.permalinkNode) {
      this.permalinkNode.addEventListener("click", event => {
        if (
          event.defaultPrevented ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey
        ) {
          return;
        }
        window.history.pushState(
          { permalink: event.target.href },
          window.title,
          event.target.href
        );
        event.preventDefault();

        updateUnpermalink();
      });
    }

    if (this.unpermalinkNode) {
      this.unpermalinkNode.addEventListener("click", event => {
        if (
          event.defaultPrevented ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey
        ) {
          return;
        }
        window.history.pushState(
          {},
          window.title,
          event.target.href
        );
        event.preventDefault();

        updateUnpermalink();
      });
    }

    for (const [name, { node }] of Object.entries(this.markdown)) {
      if (!node) {
        continue;
      }
      node.addEventListener("click", event => {
        if (
          event.defaultPrevented ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey
        ) {
          return;
        }

        this.copyMarkdown(name);

        event.preventDefault();
      });
    }

    // If the user toggles it in a different tab, update the checkbox/state here
    //
    // TODO(emilio): We should probably do the same for the case-sensitive
    // checkbox and such.
    window.addEventListener("storage", () => this.initFromLocalStorage());

    this.initFromLocalStorage();

    if (Settings.fancyBar.enabled) {
      this.addSymbolSection();
    }

    if (Settings.debug.ui) {
      this.addDebugSection();
    }
  }

  get acceleratorsEnabled() {
    return this.accelEnabledCheckbox.checked;
  }

  initFromLocalStorage() {
    let acceleratorsEnabled =
      !("accel-enable" in localStorage) ||
      localStorage.getItem("accel-enable") == "1";
    this.accelEnabledCheckbox.checked = acceleratorsEnabled;
    this.updateAccelerators();
  }

  updateAccelerators() {
    let enabled = this.acceleratorsEnabled;
    for (let accel of this.panel.querySelectorAll("span.accel")) {
      accel.style.display = enabled ? "" : "none";
    }
  }

  findAccel(key) {
    return this.panel.querySelector(`.item[data-accel="${key}"]`);
  }

  maybeHandleAccelerator(event) {
    if (!this.acceleratorsEnabled) {
      return;
    }
    if (event.altKey || event.ctrlKey || event.metaKey) {
      return;
    }
    var inputs = /input|select|textarea/i;
    if (inputs.test(event.target.nodeName)) {
      return;
    }
    let link = (() => {
      switch (event.key) {
        case "y":
        case "Y":
          return this.findAccel('Y');
        case "l":
        case "L":
          return this.findAccel('L');
        case "r":
        case "R":
          return this.findAccel('R');
        case "f":
        case "F":
          return this.findAccel('F');
        case "s":
        case "S":
          return this.findAccel('S');
        case "c":
        case "C":
          return this.findAccel('C');
      }
    })();

    if (link) {
      link.click();
      event.preventDefault();
    }
  }

  toggle() {
    let hidden = this.content.style.display != "none";
    this.content.style.display = hidden ? "none" : "";
    this.content.setAttribute("aria-hidden", hidden);
    this.content.setAttribute("aria-expanded", !hidden);
    this.icon.classList.toggle("expanded");
  }

  isExpanded() {
    return this.icon.classList.contains("expanded");
  }

  copyText(copy, text) {
    copy.setAttribute("data-copying", "true");
    navigator.clipboard
      .writeText(text)
      .then(function () {
        copy.classList.add("copied");
        setTimeout(function () {
          if (!copy.hasAttribute("data-copying")) {
            copy.classList.remove("copied");
          }
        }, 1000);
      })
      .finally(function () {
        copy.removeAttribute("data-copying");
      });
  }

  copyMarkdown(type) {
    const { node, getText } = this.markdown[type];
    if (!node || node.disabled) {
      return;
    }

    const copy = node.querySelector(".copy");
    let url = this.permalinkNode?.href || document.location.href;
    if (Settings.fancyBar.enabled) {
      url = this.reflectSelectedSymbolLineToURL(url);
    }
    const text = getText(url);

    this.copyText(copy, text);
  }

  formatSelectedLines() {
    const kPlaceholder = "...";
    const lines = [];
    let lastLine = -1;
    let commonWhitespacePrefix = null;

    function computeCommonWhitespacePrefix(lineText, existingPrefix) {
      function isWhitespace(character) {
        return character == " " || character == "\t";
      }

      if (!lineText.length) {
        // Empty lines don't contribute to the whitespace prefix.
        return existingPrefix;
      }

      // NOTE: existingPrefix === null means it's first call.
      //       existingPrefix === "" means there's no leading whitespace.
      let min = existingPrefix !== null
        ? Math.min(existingPrefix.length, lineText.length)
        : lineText.length;
      let count = 0;
      for (; count < min; ++count) {
        const inPrefix = existingPrefix
          ? existingPrefix[count] == lineText[count]
          : isWhitespace(lineText[count]);
        if (!inPrefix) {
          break;
        }
      }

      return lineText.substring(0, count);
    }

    const unsortedLines = [...Highlighter.selectedLines];
    if (Settings.fancyBar.enabled) {
      const extraLine = this.getLineNumberForSelectedSymbol();
      if (extraLine !== undefined && !unsortedLines.includes(extraLine)) {
        unsortedLines.push(extraLine);
      }
    }
    for (const line of unsortedLines.sort((a, b) => a - b)) {
      if (lastLine !== -1 && lastLine != line - 1) {
        lines.push(kPlaceholder);
      }

      const lineElem = document
        .getElementById(`line-${line}`)
        .querySelector(".source-line");
      const lineText = lineElem.textContent.replace(/\n/, "");
      commonWhitespacePrefix = computeCommonWhitespacePrefix(
        lineText,
        commonWhitespacePrefix
      );
      lines.push(lineText);
      lastLine = line;
    }

    if (commonWhitespacePrefix?.length) {
      for (let i = 0; i < lines.length; ++i) {
        if (lines[i] && lines[i] != kPlaceholder) {
          lines[i] = lines[i].substring(commonWhitespacePrefix.length);
        }
      }
    }

    return lines;
  }

  findSelectedSymbol() {
    let selectedSymbol = null;
    if (Settings.fancyBar.enabled) {
      if (ContextMenu?.selectedToken) {
        const symbols = ContextMenu.selectedToken.getAttribute("data-symbols").split(",");

        for (const sym of symbols) {
          const symInfo = SYM_INFO[sym];
          if (!symInfo || !symInfo.pretty) {
            continue;
          }

          return symInfo.pretty.replace(/[A-Za-z0-9]+ /, "");
        }
      }
    }

    return DocumentTitler?.selectedSymbol;
  }

  updateCopyState() {
    // If we're on a page without a panel, there's nothing to do.
    if (!this.panel) {
      return;
    }

    this.selectedSymbol = this.findSelectedSymbol();

    if (Settings.fancyBar.enabled) {
      if (this.copySymbolBox) {
        this.copySymbolBox.classList.toggle("disabled", !this.selectedSymbol);
      }
    }

    for (const [_, { node, isEnabled }] of Object.entries(this.markdown)) {
      if (!node) {
        continue;
      }
      if (isEnabled()) {
        node.disabled = false;
        node.removeAttribute("aria-disabled");
      } else {
        node.disabled = true;
        node.setAttribute("aria-disabled", "true");
      }
    }

    if (Settings.fancyBar.enabled) {
      this.updateSelectedSymbolView();
    }
  }

  // Add Symbol section with the symbol name and copy button.
  addSymbolSection() {
    const markdownHeader = [...this.content.querySelectorAll("h4")]
      .find(n => n.textContent == "Copy as Markdown");
    if (!markdownHeader) {
      return;
    }

    const h4 = document.createElement("h4");
    h4.textContent = "Symbol";

    markdownHeader.before(h4);

    const box = document.createElement("div");
    box.classList.add("selected-symbol-section");

    const symBox = document.createElement("div");
    symBox.classList.add("selected-symbol-box");
    this.selectedSymbolNS = document.createElement("div");
    this.selectedSymbolNS.classList.add("selected-symbol-ns");
    symBox.append(this.selectedSymbolNS);
    this.selectedSymbolLocal = document.createElement("div");
    this.selectedSymbolLocal.classList.add("selected-symbol-local");
    symBox.append(this.selectedSymbolLocal);
    box.append(symBox);

    this.copySymbolBox = document.createElement("div");
    this.copySymbolBox.classList.add("copy-box");
    const copyIndicator = document.createElement("span");
    copyIndicator.classList.add("icon", "copy", "indicator");
    const copyIcon = document.createElement("span");
    copyIcon.classList.add("icon-docs", "copy-icon");
    copyIndicator.append(copyIcon);
    const copyOk = document.createElement("span");
    copyOk.classList.add("icon-ok", "tick-icon");
    copyIndicator.append(copyOk);
    this.copySymbolBox.append(copyIndicator);
    box.append(this.copySymbolBox);

    copyIndicator.addEventListener("click", e => {
      e.preventDefault();

      if (!this.selectedSymbol) {
        return;
      }

      if (copyIndicator.hasAttribute("data-copying")) {
        return;
      }

      this.copyText(copyIndicator, this.selectedSymbol);
    });

    markdownHeader.before(box);
  }

  addDebugSection() {
    const items = [];

    const pageContent = document.getElementById("content");
    if (document.location.pathname.match(/^\/[^\/]+\/source\//) &&
        pageContent && pageContent.classList.contains("source-listing")) {
      const li = document.createElement("li");
      const link = document.createElement("a");
      link.classList.add("icon");
      link.classList.add("item");
      link.href = document.location.href.replace(/\/source\//, "/raw-analysis/");
      link.textContent = "Raw analysis records";
      li.append(link);
      items.push(li);
    }

    if (window.IS_DEBUG_LOGS_AVAILABLE) {
      const li = document.createElement("li");
      const link = document.createElement("a");
      link.classList.add("icon");
      link.classList.add("item");
      li.append(link);
      items.push(li);

      this.showHideLogsLink = link;
      this.updateDebugSectionForLocation();
    }

    this.resultsJSONBox = document.getElementById("query-debug-results-json");
    this.resultsJSONPre = document.getElementById("query-debug-results-json-pre");
    if (this.resultsJSONBox && this.resultsJSONPre) {
      const li = document.createElement("li");
      const button = document.createElement("button");
      button.classList.add("icon");
      button.classList.add("item");
      button.textContent = "Show results JSON";
      li.append(button);
      items.push(li);

      button.addEventListener("click", () => {
        if (this.resultsJSONBox.hasAttribute("aria-hidden")) {
          this.resultsJSONBox.removeAttribute("aria-hidden");
          this.resultsJSONPre.textContent = JSON.stringify(window.QUERY_RESULTS_JSON, undefined, 2);
          button.textContent = "Hide results JSON";
        } else {
          this.resultsJSONBox.setAttribute("aria-hidden", "true");
          this.resultsJSONPre.textContent = "";
          button.textContent = "Show results JSON";
        }
      });
    }

    if (items.length > 0) {
      const h4 = document.createElement("h4");
      h4.textContent = "Debug";
      this.content.append(h4);

      const ul = document.createElement("ul");
      ul.append(...items);
      this.content.append(ul);
    }
  }

  updateDebugSectionForLocation() {
    if (this.showHideLogsLink) {
      if (document.location.href.includes("&debug=true")) {
        this.showHideLogsLink.href = document.location.href.replace(/&debug=true/, "");
        this.showHideLogsLink.textContent = "Hide debug log";
      } else {
        this.showHideLogsLink.href = document.location.href + "&debug=true";
        this.showHideLogsLink.textContent = "Show debug log";
      }
    }
  }

  // Show the selected symbol's namespace prefix and the local name in the
  // Symbol section.
  updateSelectedSymbolView() {
    const sym = this.selectedSymbol || '(no symbol clicked)';
    const index = sym.lastIndexOf("::");
    let ns = "";
    let local = sym;
    if (index != -1) {
      ns = sym.slice(0, index + 2);
      local = sym.slice(index + 2);
    }
    if (this.selectedSymbolNS) {
      this.selectedSymbolNS.textContent = ns;
    }
    if (this.selectedSymbolLocal) {
      this.selectedSymbolLocal.textContent = local;
    }
  }

  // Reflect the line number of selected symbol, if any and if it's outside of
  // the selected lines.
  reflectSelectedSymbolLineToURL(spec) {
    const line = this.getLineNumberForSelectedSymbol();
    if (line === undefined) {
      return spec;
    }

    const url = new URL(spec);
    url.hash = Highlighter.toHash(line);
    return url.toString();
  }

  // Return the line number of the selected symbol if any.
  // Otherwise returns undefined.
  getLineNumberForSelectedSymbol() {
    if (!ContextMenu.selectedToken) {
      return undefined;
    }

    const containingLine = ContextMenu.selectedToken.closest(".source-line-with-number");
    if (!containingLine) {
      return undefined;
    }

    const lineNumberNode = containingLine.querySelector(".line-number");
    if (!lineNumberNode) {
      return undefined;
    }

    return parseInt(lineNumberNode.dataset.lineNumber, 10);
  }

  onSelectedLineChanged() {
    this.updateCopyState();
  }

  onSelectedSymbolChanged() {
    this.updateCopyState();
  }

  onSelectedTokenChanged() {
    this.updateCopyState();
  }

  // Returns true if the event is dispatched inside the navigation panel.
  isOnPanel(event) {
    return !!event.target.closest("#panel");
  }

  prepareForSearch() {
    // Remove any item not shared between search and other contexts.
    for (const node of [...this.content.childNodes]) {
      if (node instanceof HTMLElement) {
        if (node.classList.contains("panel-accel")) {
          continue;
        }
      }
      this.content.removeChild(node);
    }

    if (this.isExpanded()) {
      this.toggle();
    }
  }
})();

// Blurring magic based on the quite useful article by Antony Garand from
// Dec 7, 2020 at https://dev.to/antogarand/svg-metaballs-35pj that's about
// implementing metaballs in SVG with SVG filters.
//
// My initial desire for this implementation was to provide for a means of
// allowing clustering in the "neato" layout view where we fundamentally lose
// our clusters and having graphviz just draw a bounding box around things is
// insufficient.  Metaballs are a way to potentially handle that.  But
// especially with our changes to generate tables more often and neato's
// ability to handle tables, this has been less of a concern.
//
// I ended up doing the initial experimental hookup to instead try and visually
// express which nodes in the graph are "close" to the initial diagram request
// in terms of depth.  See the invocation of this method that follows it for
// more comments.
function blurrifyDiagram() {
  const diag = document.querySelector("svg");
  if (!diag) {
    return;
  }

  const createSVGElem = (name) => {
    return document.createElementNS("http://www.w3.org/2000/svg", name);
  };

  let trueDiag = diag.children[0];
  // disable the background white layer (for both modes)
  trueDiag.children[0].setAttribute("style", "display: none;")

  let blurDiag = trueDiag.cloneNode(true);
  blurDiag.setAttribute("class", "blurry depth-mode");

  // Put the filters in
  let filtRoot = createSVGElem("filter");
  filtRoot.setAttribute("id", "blur_bg");

  let filtBlurGrow = createSVGElem("feGaussianBlur");
  filtBlurGrow.setAttribute("in", "SourceGraphic");
  filtBlurGrow.setAttribute("result", "blur1");
  filtBlurGrow.setAttribute("stdDeviation", "15");
  filtRoot.appendChild(filtBlurGrow);

  let filtThresh = createSVGElem("feColorMatrix");
  filtThresh.setAttribute("in", "blur1");
  filtThresh.setAttribute("result", "matrix");
  filtThresh.setAttribute("type", "matrix");
  let transformMatrix = [
    [1, 0, 0, 0, 0],
    [0, 1, 0, 0, 0],
    [0, 0, 1, 0, 0],
    [0, 0, 0, 50, -15]
  ];
  filtThresh.setAttribute("values", transformMatrix.flat().join(" "));
  filtRoot.appendChild(filtThresh);

  let filtBlurProper = createSVGElem("feGaussianBlur");
  filtBlurProper.setAttribute("in", "matrix");
  filtBlurProper.setAttribute("result", "blur2");
  filtBlurProper.setAttribute("stdDeviation", "10");
  filtRoot.appendChild(filtBlurProper);

  diag.insertBefore(filtRoot, trueDiag);

  // Enable the filter on the blur diagram
  blurDiag.setAttribute("filter", "url(#blur_bg");

  // ### Make the normal diagram normal-ish
  trueDiag.classList.add("true-diag");

  diag.insertBefore(blurDiag, trueDiag);
}
// The initial attempt at blurring based on depth is not feeling particularly
// useful and has a wildly non-trivial performance cost, but I want to be able
// to activate it on the fly for A/B investigations, so I'm leaving it around
// so one can just invoke the command below.
//
// Hm, in fact, it seems like the performance impact is nightmarish on the
// context menu, at least when it's being shown or hidden; if you click
// elsewhere when it's already open, the performance is fine.  I wonder if the
// fact that we're cloning nodes that have identifiers which creates duplicate
// identifiers is creating a pathological situation?
//blurrifyDiagram();

// In order to provide more useful click/hover targets for diagram edges, we
// duplicate line body "path" element to create one with a wider stroke that is
// not visible.
function makeDiagramHoverEdges() {
  const diag = document.querySelector("svg");
  if (!diag) {
    return;
  }

  const edges = diag.querySelectorAll("g.edge > path");
  for (const path of edges) {
    const dupe = path.cloneNode(false);
    dupe.classList.add("clicktarget");
    // let's insert the clicktarget after the actual path so it is always what
    // the hit test finds.
    path.insertAdjacentElement("afterend", dupe);
  }
}
makeDiagramHoverEdges();

// Scroll the first root node of the diagram so that it's centered.  Through use
// of `?.` this won't freak out if there are no matches.  According to
// performance.now(), this takes 3ms on the 20,765 line indexedDB/ActorsParent.cpp
// right now.
//
// This file is loaded at the bottom of the HTML file so the DOM is available,
// although I'm not entirely sure this is wise versus hooking the load event.
//
// TODO: Be more wise.
document.querySelector(".diagram-depth-0 polygon")?.scrollIntoView({
  behavior: "instant",
  block: "center",
  inline: "center"
});
