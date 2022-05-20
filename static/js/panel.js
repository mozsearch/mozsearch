var Panel = new (class Panel {
  constructor() {
    this.panel = document.getElementById("panel");
    this.toggleButton = document.getElementById("panel-toggle");
    this.icon = this.panel.querySelector(".navpanel-icon");
    this.content = document.getElementById("panel-content");
    this.accelEnabledCheckbox = document.getElementById("panel-accel-enable");

    this.permalinkNode = this.findItem("Permalink");
    this.logNode = this.findItem("Log");
    this.rawNode = this.findItem("Raw");

    this.markdown = {
      "filename": {
        node: this.findItem("Filename Link"),
        isEnabled: () => {
          return true;
        },
        getText: url => {
          const filename = new URL(url).pathname.match(/\/([^\/]+)$/)[1];
          return `[${filename}](${url})`;
        },
      },
      "symbol": {
        node: this.findItem("Symbol Link"),
        isEnabled: () => {
          return DocumentTitler?.selectedSymbol;
        },
        getText: url => {
          return `[${DocumentTitler.selectedSymbol}](${url})`;
        },
      },
      "block": {
        node: this.findItem("Code Block"),
        isEnabled: () => {
          return Highlight?.selectedLines.size > 0;
        },
        getText: url => {
          const file = document.getElementById("file");
          const lang = file.getAttribute("data-markdown-slug") || "";
          return [url, "```" + lang, ...this.formatSelectedLines(), "```"].join("\n");
        },
      },
    };

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

    if (this.permalinkNode) {
      this.permalinkNode.addEventListener("click", event => {
        if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
          return;
        }
        window.history.pushState(
          { permalink: event.target.href },
          window.title,
          event.target.href
        );
        event.preventDefault();
      });
    }

    for (const [name, { node }] of Object.entries(this.markdown)) {
      if (!node) {
        continue;
      }
      node.addEventListener("click", event => {
        if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
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
    navigator.clipboard.writeText(text)
      .then(function() {
        copy.classList.add("copied");
        setTimeout(function() {
          if (!copy.hasAttribute("data-copying")) {
            copy.classList.remove("copied");
          }
        }, 1000);
      })
      .finally(function() {
        copy.removeAttribute("data-copying");
      });
  }

  copyMarkdown(type) {
    const { node, getText } = this.markdown[type];
    if (node.disabled) {
      return;
    }

    const copy = node.querySelector(".copy");
    const url = this.permalinkNode?.href || document.location.href;
    const text = getText(url);;

    this.copyText(copy, text);
  }

  formatSelectedLines() {
    const texts = [];
    let lastLine = -1;
    for (const line of [...Highlight.selectedLines].sort((a, b) => a - b)) {
      if (lastLine !== -1 && lastLine != line - 1) {
        texts.push("...");
      }

      const lineElem = document.getElementById(`line-${line}`).querySelector(".source-line");
      texts.push(lineElem.textContent.replace(/\n/, ""));

      lastLine = line;
    }
    return texts;
  }

  updateMarkdownState() {
    for (const [_, { node, isEnabled }] of Object.entries(this.markdown)) {
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
