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

    this.selectedLines = [];
    this.selectedSymbol = "";
    this.markdown = {
      "filename": {
        node: this.findItem("Filename Link"),
        isEnabled: () => {
          return this.selectedLines.length > 0;
        },
        getText: (url, filename) => {
          return `[${filename}](${url})`;
        },
      },
      "symbol": {
        node: this.findItem("Symbol Link"),
        isEnabled: () => {
          return this.selectedSymbol;
        },
        getText: (url, filename) => {
          return `[${this.selectedSymbol}](${url})`;
        },
      },
      "block": {
        node: this.findItem("Code Block"),
        isEnabled: () => {
          return this.selectedLines.length > 0;
        },
        getText: (url, filename) => {
          const lang = this.getLanguageFor(filename);
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

    for (let copy of this.panel.querySelectorAll(".copy")) {
      copy.addEventListener("click", e => {
        e.preventDefault();

        for (const [name, { node }] of Object.entries(this.markdown)) {
          if (copy.parentNode == node) {
            this.copyMarkdown(name);
            return;
          }
        }

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
      if (node) {
        node.addEventListener("click", event => {
          if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
            return;
          }

          this.copyMarkdown(name);
          
          event.preventDefault();
        });
      }
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
          break;
        case "s":
        case "S":
          return this.markdown.symbol.node;
          break;
        case "c":
        case "C":
          return this.markdown.block.node;
          break;
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
    if (node.classList.contains("disabled")) {
      return;
    }

    const copy = node.querySelector(".copy");
    const url = this.permalinkNode ? this.permalinkNode.href : document.location.href;
    const filename = new URL(url).pathname.match(/\/([^\/]+)$/)[1];
    const text = getText(url, filename);;

    this.copyText(copy, text);
  }

  getLanguageFor(filename) {
    filename = filename.replace(/\.in$/, "");

    const langs = {
      // suffix => language
      ".c": "c",
      ".cc": "cpp",
      ".configure": "python",
      ".cpp": "cpp",
      ".css": "css",
      ".diff": "diff",
      ".h": "cpp",
      ".headers": "http",
      ".hh": "cpp",
      ".hpp": "cpp",
      ".htm": "html",
      ".html": "html",
      ".java": "java",
      ".js": "js",
      ".jsm": "js",
      ".json": "json",
      ".jsx": "js",
      ".m": "c",
      ".mathml": "mathml",
      ".md": "md",
      ".mjs": "js",
      ".mm": "cpp",
      ".mozbuild": "py",
      ".patch": "diff",
      ".pl": "perl",
      ".py": "python",
      ".rs": "rust",
      ".rst": "rest",
      ".scss": "css",
      ".sjs": "js",
      ".svg": "xml",
      ".toml": "toml",
      ".ts": "js",
      ".xht": "xhtml",
      ".xhtml": "xhtml",
      ".xml": "xml",
      ".xul": "xul",
      ".yaml": "yaml",
      ".yml": "yaml",
      "^headers^": "http",
      "moz.build": "py",
    };

    for (const [suffix, lang] of Object.entries(langs)) {
      if (filename.endsWith(suffix)) {
        return lang;
      }
    }

    return "";
  }

  formatSelectedLines() {
    const texts = [];
    let lastLine = -1;
    for (const line of this.selectedLines) {
      if (lastLine !== -1 && lastLine != line - 1) {
        texts.push("...");
      }

      const lineElem = document.getElementById(`line-${line}`).querySelector(".source-line");
      texts.push(lineElem.textContent.replace(/\n/g, ""));

      lastLine = line;
    }
    return texts;
  }

  updateMarkdownState() {
    for (const [_, { node, isEnabled }] of Object.entries(this.markdown)) {
      if (isEnabled()) {
        node.classList.remove("disabled");
        node.removeAttribute("aria-disabled");
        node.querySelector(".copy").disabled = false;
      } else {
        node.classList.add("disabled");
        node.setAttribute("aria-disabled", "true");
        node.querySelector(".copy").disabled = true;
      }
    }
  }

  onSelectedLineChanged(selectedLines) {
    this.selectedLines = [...selectedLines].sort((a, b) => a - b);
    this.updateMarkdownState();
  }

  onSelectedSymbolChanged(selectedSymbol) {
    this.selectedSymbol = selectedSymbol;
    this.updateMarkdownState();
  }
})();
