var Panel = new (class Panel {
  constructor() {
    this.panel = document.getElementById("panel");
    // Avoid complaining if this page doesn't have a panel on it.
    if (!this.panel) {
      return;
    }
    this.toggleButton = document.getElementById("panel-toggle");
    this.icon = this.panel.querySelector(".navpanel-icon");
    this.settingsButton = document.getElementById("show-settings");
    this.content = document.getElementById("panel-content");
    this.accelEnabledCheckbox = document.getElementById("panel-accel-enable");

    this.permalinkNode = this.findItem("Permalink");
    this.unpermalinkNode = this.findItem("Remove the Permalink");
    this.logNode = this.findItem("Log");
    this.rawNode = this.findItem("Raw");

    this.markdown = {
      filename: {
        node: this.findItem("Filename Link"),
        isEnabled: () => {
          return true;
        },
        getText: url => {
          const filename = new URL(url).pathname.match(/\/([^\/]+)$/)[1];
          return `[${filename}](${url})`;
        },
      },
      symbol: {
        node: this.findItem("Symbol Link"),
        isEnabled: () => {
          return DocumentTitler?.selectedSymbol;
        },
        getText: url => {
          return `[${DocumentTitler.selectedSymbol}](${url})`;
        },
      },
      block: {
        node: this.findItem("Code Block"),
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

  findItem(title) {
    return this.panel.querySelector(`.item[title="${title}"]`);
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
          return this.permalinkNode;
        case "l":
        case "L":
          return this.logNode;
        case "r":
        case "R":
          return this.rawNode;
        case "f":
        case "F":
          return this.markdown.filename.node;
        case "s":
        case "S":
          return this.markdown.symbol.node;
        case "c":
        case "C":
          return this.markdown.block.node;
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
    const url = this.permalinkNode?.href || document.location.href;
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

    for (const line of [...Highlighter.selectedLines].sort((a, b) => a - b)) {
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

  updateMarkdownState() {
    // If we're on a page without a panel, there's nothing to do.
    if (!this.panel) {
      return;
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
  }

  onSelectedLineChanged() {
    this.updateMarkdownState();
  }

  onSelectedSymbolChanged() {
    this.updateMarkdownState();
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
