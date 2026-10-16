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

    // The formats of "Copy as" (see format.rs's
    // create_markdown_panel_section), by name: each one's button, whether it
    // applies, its name and description for the popup, and its text for a URL.
    const filenameOf = url => new URL(url).pathname.match(/\/([^\/]+)$/)[1];
    // (A symbol link's text: the selected symbol, or the file and its
    // selected lines.)
    const linkText = url => this.selectedSymbol || `${filenameOf(url)}${new URL(url).hash}`;
    const codeBlock = () => {
      const file = document.getElementById("file");
      const lang = file.getAttribute("data-markdown-slug") || "";
      return ["```" + lang, ...this.formatSelectedLines(), "```"];
    };
    const hasLines = () => Highlighter?.selectedLines.size > 0;
    this.markdown = {
      symbol: {
        node: document.querySelector("#panel-copy-symbol-link"),
        name: "Symbol link",
        description: "For mentioning the selected symbol (or the file and its selected lines) in prose.",
        isEnabled: () => true,
        getText: url => `[${linkText(url)}](${url})`,
      },
      symbolBlock: {
        node: document.querySelector("#panel-copy-symbol-block"),
        name: "Symbol link and code block",
        description: "The selected lines, led by the link to their symbol.",
        isEnabled: hasLines,
        getText: url => [`[${linkText(url)}](${url})`, ...codeBlock()].join("\n"),
      },
      block: {
        node: document.querySelector("#panel-copy-code-block"),
        name: "Code block",
        description: "The selected lines, led by their URL.",
        isEnabled: hasLines,
        getText: url => [url, ...codeBlock()].join("\n"),
      },
      filename: {
        node: document.querySelector("#panel-copy-filename-link"),
        name: "File link",
        description: "A link to the file, with its name as its text.",
        isEnabled: () => true,
        getText: url => `[${filenameOf(url)}](${url})`,
      },
    };
    this.copyAsButton = document.querySelector("#panel-copy-as");
    this.copyAsButton?.addEventListener("click", () => this.toggleCopyAsPopup());

    // We want the default event for clicking on the settings link to work, but
    // we want to stop its propagation so that it doesn't bubble up to the
    // toggle button handler that comes next.
    this.settingsButton.addEventListener("click", (evt) => { evt.stopPropagation(); });

    this.toggleButton.addEventListener("click", () => this.toggle());
    // (The keyboard shortcuts' setting is on the settings page.)
    this.accelEnabledCheckbox?.addEventListener("change", event => {
      localStorage.setItem("accel-enable", event.target.checked ? "1" : "0");
      this.updateAccelerators();
    });
    document.documentElement.addEventListener("keypress", event =>
      this.maybeHandleAccelerator(event)
    );

    // An info box's "Details" button shows its panel section (ex: a test's,
    // see info_boxes/test.liquid), opening the panel if it's collapsed.
    for (const button of document.querySelectorAll("button.info-box-details")) {
      button.addEventListener("click", () => {
        if (!this.isExpanded()) {
          this.toggle();
        }
        const section = document.getElementById(button.dataset.panelSection);
        section?.scrollIntoView({ block: "start", behavior: "smooth" });
        section?.classList.add("panel-section-flash");
        setTimeout(() => section?.classList.remove("panel-section-flash"), 1000);
      });
    }

    for (let copy of this.panel.querySelectorAll("button.copy")) {
      copy.addEventListener("click", e => {
        e.preventDefault();

        if (copy.hasAttribute("data-copying")) {
          return;
        }

        // (Its text, if not its item's link, ex: a review skill's path.)
        // (The permalink's, with the selected token; see permalinkURL.)
        const text = copy.parentNode === this.permalinkNode
          ? this.permalinkURL(copy.parentNode.href)
          : copy.dataset.copy ?? copy.parentNode.href;
        this.copyText(copy, text);
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
        const href = this.permalinkURL(event.target.href);
        window.history.pushState(
          { permalink: href },
          window.title,
          href
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

    window.addEventListener("resize", () => this.maybeAutoCollapse());

    this.initFromLocalStorage();

    this.maybeAutoCollapse();

    if (Settings.fancyBar.enabled) {
      this.setupFancyBar();
      this.addSymbolSection();
      this.addInsideSection();
      this.addOpenInEditorAction();
    }

    if (Settings.navPanel.showLenses) {
      this.addLensesSection();
    }

    if (Settings.debug.ui) {
      this.addDebugSection();
    }
  }

  get acceleratorsEnabled() {
    return !("accel-enable" in localStorage) || localStorage.getItem("accel-enable") == "1";
  }

  initFromLocalStorage() {
    if (this.accelEnabledCheckbox) {
      this.accelEnabledCheckbox.checked = this.acceleratorsEnabled;
    }
    this.updateAccelerators();
  }

  updateAccelerators() {
    let enabled = this.acceleratorsEnabled;
    for (let accel of this.panel.querySelectorAll("span.accel")) {
      accel.style.display = enabled ? "" : "none";
    }
  }

  findAccel(key) {
    return this.panel.querySelector(`.item[data-accel="${CSS.escape(key)}"]`);
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
        case "\\":
        case "¥":
          return this.findAccel("\\");
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
        case "b":
        case "B":
          return this.findAccel('B');
        case "t":
        case "T":
          return this.findAccel('T');
      }
    })();

    if (link) {
      if (link.classList.contains("disabled")) {
        return;
      }
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
    this.updateFancyBarOpen();
  }

  /**
   * The fancy bar: the panel docked as a sidebar on the right, below the
   * header, whose width, which the page leaves to it while it's expanded,
   * can be changed by dragging its left edge (or with the arrow keys there),
   * and is kept for later pages.
   */
  setupFancyBar() {
    const root = document.documentElement;
    root.classList.add("fancy-bar");
    const stored = parseInt(localStorage.getItem("fancy-bar-width"), 10);
    if (stored > 0) {
      root.style.setProperty("--fancy-bar-width", `${stored}px`);
    }

    const resizer = document.createElement("div");
    resizer.className = "fancy-bar-resizer";
    resizer.setAttribute("role", "separator");
    resizer.setAttribute("aria-orientation", "vertical");
    resizer.setAttribute("aria-label", "Resize the navigation panel");
    resizer.tabIndex = 0;
    document.body.append(resizer);
    this.fancyBarResizer = resizer;

    const setWidth = width => {
      const clamped = Math.round(Math.max(200, Math.min(width, window.innerWidth * 0.6)));
      root.style.setProperty("--fancy-bar-width", `${clamped}px`);
      return clamped;
    };
    const save = () => {
      localStorage.setItem("fancy-bar-width", String(Math.round(this.panel.getBoundingClientRect().width)));
    };
    resizer.addEventListener("pointerdown", event => {
      event.preventDefault();
      resizer.setPointerCapture(event.pointerId);
      root.classList.add("fancy-bar-resizing");
    });
    resizer.addEventListener("pointermove", event => {
      if (resizer.hasPointerCapture(event.pointerId)) {
        setWidth(window.innerWidth - event.clientX);
      }
    });
    const stop = event => {
      if (resizer.hasPointerCapture(event.pointerId)) {
        resizer.releasePointerCapture(event.pointerId);
        root.classList.remove("fancy-bar-resizing");
        save();
      }
    };
    resizer.addEventListener("pointerup", stop);
    resizer.addEventListener("pointercancel", stop);
    resizer.addEventListener("keydown", event => {
      const step = { ArrowLeft: 20, ArrowRight: -20 }[event.key];
      if (step) {
        event.preventDefault();
        setWidth(this.panel.getBoundingClientRect().width + step);
        save();
      }
    });

    this.updateFancyBarOpen();
  }

  updateFancyBarOpen() {
    document.documentElement.classList.toggle(
      "fancy-bar-open",
      document.documentElement.classList.contains("fancy-bar") && this.isExpanded()
    );
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

    // (The compact buttons say they copied themselves.)
    const copy = node.querySelector(".copy") || node;
    this.copyText(copy, getText(this.copyURL()));
  }

  /** The URL that "Copy as" copies: the permalink's, with the selection. */
  copyURL() {
    let url = this.permalinkNode?.href || document.location.href;
    if (Settings.fancyBar.enabled) {
      url = this.reflectSelectedSymbolLineToURL(url);
    }
    return this.permalinkURL(url);
  }

  /**
   * A permalink with the selected token, if any (as `?token=`, the line hash
   * staying as it is), which a later page of the file at the permalink's
   * revision can follow into the future (see file-timeline.js's
   * PermalinkFuture).
   */
  permalinkURL(spec) {
    if (!this.selectedTokenIndex || !/\/rev\//.test(spec)) {
      return spec;
    }
    const url = new URL(spec);
    url.searchParams.set("token", this.selectedTokenIndex);
    return url.toString();
  }

  /**
   * The selected token's index in the file, by its hyperblame data (see
   * HyperblameData's tokens' `start`, `end` and `index`), for permalinks.
   */
  async updateSelectedTokenIndex() {
    const token = typeof ContextMenu !== "undefined" ? ContextMenu.selectedToken : null;
    this.selectedTokenIndex = null;
    const data = typeof HyperblameContexts !== "undefined" ? HyperblameContexts.contexts?.[0] : null;
    const code = token?.closest("code.source-line");
    const row = code?.closest(".source-line-with-number");
    const lineno = parseInt(row?.querySelector(".line-number")?.dataset.lineNumber, 10);
    if (!data || !code || !lineno) {
      return;
    }
    let offset = 0;
    const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
    while (walker.nextNode() && !token.contains(walker.currentNode)) {
      offset += walker.currentNode.length;
    }
    try {
      const tokens = await data.getLineTokens(lineno);
      if (ContextMenu.selectedToken !== token) {
        return;
      }
      const found = (tokens || []).find(t => t.start <= offset && offset < t.end);
      this.selectedTokenIndex = found?.index ?? null;
    } catch (e) {
      this.selectedTokenIndex = null;
    }
  }

  /**
   * "Copy as...": a popup of the formats, each previewed as it'd be copied
   * now, with a button to copy it, and the raw file, to open or copy its URL.
   */
  toggleCopyAsPopup() {
    if (this.copyAsPopup) {
      this.closeCopyAsPopup();
      return;
    }
    const anchor = this.copyAsButton;
    const popup = document.createElement("div");
    popup.className = "lens-popup copy-as-popup";
    popup.setAttribute("role", "dialog");
    popup.setAttribute("aria-label", "Copy as");
    const heading = document.createElement("h5");
    heading.textContent = "Copy as";
    popup.append(heading);

    const url = this.copyURL();
    const list = document.createElement("ul");
    const addRow = (name, description, text, accel, copy) => {
      const item = document.createElement("li");
      const title = document.createElement("div");
      title.className = "copy-as-title";
      const strong = document.createElement("strong");
      strong.textContent = name;
      title.append(strong);
      if (accel) {
        const key = document.createElement("span");
        key.className = "accel";
        key.textContent = accel;
        title.append(" ", key);
      }
      if (copy) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "copy-as-copy";
        button.textContent = "Copy";
        button.addEventListener("click", () => copy(button));
        title.append(button);
      }
      const desc = document.createElement("div");
      desc.className = "copy-as-description";
      desc.textContent = description;
      item.append(title, desc);
      if (text !== null) {
        // (Its Markdown on the left, and how it renders on the right.)
        const columns = document.createElement("div");
        columns.className = "copy-as-columns";
        const pre = document.createElement("pre");
        pre.className = "copy-as-preview";
        pre.textContent = text;
        columns.append(pre, this.renderMarkdownPreview(text));
        item.append(columns);
      }
      list.append(item);
    };
    for (const [name, format] of Object.entries(this.markdown)) {
      if (!format.node) {
        continue;
      }
      const enabled = format.isEnabled();
      addRow(format.name, enabled ? format.description : `${format.description} (Select some lines first.)`,
             enabled ? format.getText(url) : null, format.node.dataset.accel,
             enabled ? button => this.copyText(button, format.getText(url)) : null);
    }
    const raw = document.querySelector("#panel-raw");
    if (raw) {
      addRow("Raw file", "The file's contents, without searchfox's markup.", raw.href, "R",
             button => this.copyText(button, raw.href));
      const open = document.createElement("a");
      open.href = raw.href;
      open.textContent = "Open the raw file";
      list.lastChild.append(open);
    }
    popup.append(list);

    const close = document.createElement("button");
    close.type = "button";
    close.className = "lens-popup-close";
    close.textContent = "Close";
    close.addEventListener("click", () => this.closeCopyAsPopup());
    popup.append(close);

    document.body.append(popup);
    this.copyAsPopup = popup;
    anchor.setAttribute("aria-expanded", "true");
    // (Beside the panel, like the blame lens's.)
    const rect = anchor.getBoundingClientRect();
    const left = Math.min(rect.left, this.panel.getBoundingClientRect().left);
    popup.style.right = `${Math.max(8, window.innerWidth - left + 8)}px`;
    popup.style.top = `${Math.max(8, Math.min(rect.top, window.innerHeight - popup.offsetHeight - 8))}px`;
    popup.querySelector("button")?.focus();

    this.copyAsDismiss = event => {
      if (event.type === "keydown" ? event.key === "Escape"
          : !popup.contains(event.target) && !anchor.contains(event.target)) {
        this.closeCopyAsPopup();
      }
    };
    document.addEventListener("keydown", this.copyAsDismiss);
    document.addEventListener("pointerdown", this.copyAsDismiss, true);
  }

  /**
   * The rendering of a "Copy as" format's Markdown, for its preview: its links
   * (`[text](url)`, and bare URLs) and fenced code blocks, which are all the
   * formats have.
   */
  renderMarkdownPreview(text) {
    const out = document.createElement("div");
    out.className = "copy-as-rendered";
    const inline = (line, parent) => {
      const re = /\[([^\]]*)\]\(([^)\s]+)\)|(https?:\/\/\S+)/g;
      let last = 0;
      let m;
      while ((m = re.exec(line))) {
        parent.append(line.slice(last, m.index));
        const a = document.createElement("a");
        a.href = m[2] ?? m[3];
        a.textContent = m[1] ?? m[3];
        parent.append(a);
        last = re.lastIndex;
      }
      parent.append(line.slice(last));
    };
    const lines = text.split("\n");
    let paragraph = null;
    for (let i = 0; i < lines.length; i++) {
      if (/^```/.test(lines[i])) {
        const code = [];
        for (i++; i < lines.length && lines[i] !== "```"; i++) {
          code.push(lines[i]);
        }
        const pre = document.createElement("pre");
        const codeElt = document.createElement("code");
        codeElt.textContent = code.join("\n");
        pre.append(codeElt);
        out.append(pre);
        paragraph = null;
        continue;
      }
      if (paragraph) {
        paragraph.append(" ");
      } else {
        paragraph = document.createElement("p");
        out.append(paragraph);
      }
      inline(lines[i], paragraph);
    }
    return out;
  }

  closeCopyAsPopup() {
    this.copyAsPopup?.remove();
    this.copyAsPopup = null;
    this.copyAsButton?.setAttribute("aria-expanded", "false");
    document.removeEventListener("keydown", this.copyAsDismiss);
    document.removeEventListener("pointerdown", this.copyAsDismiss, true);
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
      .find(n => n.textContent == "Copy as");
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

  /**
   * The "Inside" section says what the hovered line (or else the selected
   * one) is inside, from the source listing's nesting containers (whose first
   * lines are the sticky lines), outermost first, each linking to its line,
   * ex: "tasks › linux64/opt › fetches › toolchain" in a taskgraph kind.  (A
   * line starting a nesting container isn't inside it.)
   */
  addInsideSection() {
    const file = document.getElementById("file");
    const markdownHeader = [...this.content.querySelectorAll("h4")]
      .find(n => n.textContent == "Copy as");
    if (!file || !markdownHeader || !file.querySelector(".nesting-container")) {
      return;
    }

    const h4 = document.createElement("h4");
    h4.textContent = "Inside";
    this.insideBox = document.createElement("div");
    this.insideBox.id = "panel-inside";
    this.insideBox.className = "inside-section";
    markdownHeader.before(h4, this.insideBox);

    file.addEventListener("mouseover", event => {
      const line = event.target.closest?.(".source-line-with-number");
      if (line) {
        this.showInside(line);
      }
    });
    // (The selected line's once code-highlighter.js has selected it.)
    const showSelected = () => {
      const selected = window.Highlighter?.selectedLines?.size > 0
        ? document.getElementById(`line-${Math.min(...Highlighter.selectedLines)}`)
        : null;
      this.showInside(selected);
    };
    file.addEventListener("mouseleave", showSelected);
    if (document.readyState == "loading") {
      document.addEventListener("DOMContentLoaded", showSelected);
    } else {
      showSelected();
    }
  }

  /** Say what `line` (a source listing's row, if any) is inside. */
  showInside(line) {
    if (line === this.insideLine && line) {
      return;
    }
    this.insideLine = line;
    const crumbs = [];
    let container = line?.closest(".nesting-container");
    if (container && line.parentElement === container && line.classList.contains("nesting-sticky-line")) {
      container = container.parentElement.closest(".nesting-container");
    }
    for (; container; container = container.parentElement.closest(".nesting-container")) {
      const sticky = container.querySelector(":scope > .nesting-sticky-line");
      if (!sticky) {
        continue;
      }
      const text = sticky.querySelector(".source-line")?.textContent.trim() ?? "";
      const pretty = window.SYM_INFO?.[container.dataset.nestingSym]?.pretty;
      // (Pretty names can have their syntax's kind first, ex: "class Foo".)
      let label = pretty
        ? pretty.replace(/^[\w-]+ /, "").split("::").pop()
        : text.replace(/\s+/g, " ").replace(/\s*[{:]$/, "");
      if (label.length > 60) {
        label = `${label.slice(0, 59)}\u2026`;
      }
      const lineno = sticky.id.replace(/^line-/, "");
      const link = document.createElement("a");
      link.href = `#${lineno}`;
      link.title = `Line ${lineno}: ${text}`;
      link.textContent = label;
      crumbs.unshift(link);
    }
    if (!line) {
      this.insideBox.replaceChildren();
    } else if (crumbs.length == 0) {
      const top = document.createElement("span");
      top.className = "inside-top";
      top.textContent = "(the top level)";
      this.insideBox.replaceChildren(top);
    } else {
      this.insideBox.replaceChildren(...crumbs.flatMap((crumb, i) => i == 0 ? [crumb] : [" \u203a ", crumb]));
    }
  }

  /**
   * The "Lenses" section lets the user change how the page presents
   * information, currently how the token-centric blame strip is colored.
   * Changes apply to this page immediately and become the setting for pages
   * loaded later, but pages which are already loaded don't change.
   */
  addLensesSection() {
    const items = [];

    // (Diffs have `BLAME_INFOS`, and pages of several rendered documents'
    // sections `RENDERED_BLAME_INFOS`; see `HyperblameContexts` in
    // hyperblame.js.)
    if (typeof BLAME_INFO !== "undefined" || typeof BLAME_INFOS !== "undefined" ||
        typeof RENDERED_BLAME_INFOS !== "undefined") {
      items.push(this.createBlameLens());
    }

    if (items.length > 0) {
      const h4 = document.createElement("h4");
      h4.textContent = "Lenses";
      this.content.append(h4);

      const ul = document.createElement("ul");
      ul.append(...items);
      this.content.append(ul);
    }
  }

  /**
   * The blame lens: "Blame..." for a popup explaining the blame strip's color
   * modes, with their settings, and beside it a button for each mode.
   */
  createBlameLens() {
    const li = document.createElement("li");
    li.classList.add("lens", "lens-blame");

    const more = document.createElement("button");
    more.type = "button";
    more.id = "panel-lens-blame";
    more.className = "lens-more";
    more.textContent = "Blame\u2026";
    more.title = "How the blame strip can be colored, and its settings";
    more.setAttribute("aria-haspopup", "dialog");
    more.setAttribute("aria-expanded", "false");
    more.addEventListener("click", () => this.toggleBlamePopup(more));

    const modes = document.createElement("span");
    modes.className = "lens-modes";
    modes.setAttribute("role", "group");
    modes.setAttribute("aria-label", "Blame strip colors");
    const SHORT = { alternating: "alt", age: "age", author: "author" };
    this.blameModeButtons = SETTING_DEFS.blame.colorMode.choices.map(([mode, label]) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = SHORT[mode] ?? label;
      button.title = label;
      button.dataset.mode = mode;
      button.addEventListener("click", () => this.setBlameMode(mode));
      return button;
    });
    modes.append(...this.blameModeButtons);
    this.updateBlameModeButtons();

    li.append(more, modes);
    return li;
  }

  setBlameMode(mode) {
    BlameColorizer.apply(mode);
    Settings.__setValueFromIdSpace("blame--color-mode", mode);
    this.updateBlameModeButtons();
  }

  updateBlameModeButtons() {
    const mode = Settings.blame.colorMode;
    for (const button of this.blameModeButtons) {
      button.setAttribute("aria-pressed", String(button.dataset.mode === mode));
    }
    for (const radio of this.blamePopup?.querySelectorAll("input[name=blame-mode]") ?? []) {
      radio.checked = radio.value === mode;
    }
  }

  toggleBlamePopup(anchor) {
    if (this.blamePopup) {
      this.closeBlamePopup();
      return;
    }
    const DESCRIPTIONS = {
      alternating: "Two shades, switching where the commit changes, like classic blame.",
      age: "The more recent the commit, the darker the color, by the age scale below.",
      author: "A color for each author of a commit (the same colors for several, if there are many).",
    };

    const popup = document.createElement("div");
    popup.className = "lens-popup";
    popup.setAttribute("role", "dialog");
    popup.setAttribute("aria-label", "Blame strip colors");

    const heading = document.createElement("h5");
    heading.textContent = "Blame strip colors";
    const intro = document.createElement("p");
    intro.textContent = "The strip beside the code shows the commits that last changed its tokens; " +
      "hover over it or click it for them.  Its colors can show:";
    popup.append(heading, intro);

    const list = document.createElement("ul");
    for (const [mode, label] of SETTING_DEFS.blame.colorMode.choices) {
      const item = document.createElement("li");
      const radioLabel = document.createElement("label");
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "blame-mode";
      radio.value = mode;
      radio.addEventListener("change", () => this.setBlameMode(mode));
      const name = document.createElement("strong");
      name.textContent = label;
      radioLabel.append(radio, name, `: ${DESCRIPTIONS[mode] ?? ""}`);
      item.append(radioLabel);
      if (mode === "age") {
        const scaleLabel = document.createElement("label");
        scaleLabel.className = "lens-setting";
        scaleLabel.textContent = "Age scale: ";
        const scale = document.createElement("select");
        scale.id = "blame--age-scale";
        scale.append(...Settings.createChoiceOptions(SETTING_DEFS.blame.ageScale.choices));
        scale.value = Settings.blame.ageScale;
        scaleLabel.append(scale);
        const legend = document.createElement("ul");
        legend.className = "lens-age-legend";
        const updateLegend = () => {
          legend.replaceChildren(...BlameColorizer.ageBucketLabels().map((text, i) => {
            const swatch = document.createElement("li");
            swatch.style.setProperty("--swatch", `var(--blame-age-${i})`);
            swatch.textContent = text;
            return swatch;
          }));
        };
        scale.addEventListener("change", () => {
          Settings.__setValueFromIdSpace("blame--age-scale", scale.value);
          BlameColorizer.apply(Settings.blame.colorMode);
          updateLegend();
        });
        updateLegend();
        item.append(scaleLabel, legend);
      }
      list.append(item);
    }
    popup.append(list);

    const close = document.createElement("button");
    close.type = "button";
    close.className = "lens-popup-close";
    close.textContent = "Close";
    close.addEventListener("click", () => this.closeBlamePopup());
    popup.append(close);

    document.body.append(popup);
    this.blamePopup = popup;
    this.blamePopupAnchor = anchor;
    anchor.setAttribute("aria-expanded", "true");
    this.updateBlameModeButtons();

    // Beside the panel, at the anchor's height, as far as it fits.
    const rect = anchor.getBoundingClientRect();
    const left = Math.min(rect.left, this.panel.getBoundingClientRect().left);
    popup.style.right = `${Math.max(8, window.innerWidth - left + 8)}px`;
    const top = Math.min(rect.top, window.innerHeight - popup.offsetHeight - 8);
    popup.style.top = `${Math.max(8, top)}px`;
    popup.querySelector("input:checked, input")?.focus();

    this.blamePopupDismiss = event => {
      if (event.type === "keydown" ? event.key === "Escape"
          : !popup.contains(event.target) && !anchor.contains(event.target)) {
        this.closeBlamePopup();
      }
    };
    document.addEventListener("keydown", this.blamePopupDismiss);
    document.addEventListener("pointerdown", this.blamePopupDismiss, true);
  }

  closeBlamePopup() {
    this.blamePopup?.remove();
    this.blamePopup = null;
    this.blamePopupAnchor?.setAttribute("aria-expanded", "false");
    document.removeEventListener("keydown", this.blamePopupDismiss);
    document.removeEventListener("pointerdown", this.blamePopupDismiss, true);
  }

  /**
   * The analysis records widget: the hovered token's records (those of its
   * line overlapping it), or its line's, rendered like devtools' objects, from
   * the file's raw analysis (fetched on the first hover).  It keeps its
   * records while the context menu is open (ex: for a clicked token).  Its
   * "Properties" button picks which properties it shows.
   */
  createRecordsWidget(rawUrl) {
    const li = document.createElement("li");
    li.className = "debug-records";
    const header = document.createElement("div");
    header.className = "debug-records-header";
    header.textContent = "Analysis records";
    const propsButton = document.createElement("button");
    propsButton.type = "button";
    propsButton.textContent = "Properties";
    propsButton.title = "Choose which properties of the records to show";
    header.append(propsButton);
    const props = document.createElement("div");
    props.className = "debug-records-props";
    props.hidden = true;
    const body = document.createElement("div");
    body.className = "debug-records-body";
    body.textContent = "Hover over a token or a line for its records.";
    li.append(header, props, body);

    const hiddenKey = "debug-records-hidden";
    const hidden = new Set((localStorage.getItem(hiddenKey) || "").split(",").filter(Boolean));
    let byLine = null;
    let loading = null;
    let last = null;

    const load = () => {
      loading ??= fetch(rawUrl).then(r => r.text()).then(text => {
        byLine = new Map();
        for (const line of text.split("\n")) {
          if (!line.trim()) {
            continue;
          }
          let record;
          try {
            record = JSON.parse(line);
          } catch (e) {
            continue;
          }
          const m = /^(\d+):(\d+)(?:-(?:(\d+):)?(\d+))?$/.exec(record.loc || "");
          if (!m) {
            continue;
          }
          const lineno = parseInt(m[1], 10);
          const start = parseInt(m[2], 10);
          const end = m[4] !== undefined && m[3] === undefined ? parseInt(m[4], 10) : Infinity;
          if (!byLine.has(lineno)) {
            byLine.set(lineno, []);
          }
          byLine.get(lineno).push({ start, end, record });
        }
      }).catch(e => {
        body.textContent = `Couldn't load the records: ${e}`;
      });
      return loading;
    };

    const renderValue = value => {
      if (value !== null && typeof value === "object") {
        const details = document.createElement("details");
        const summary = document.createElement("summary");
        summary.textContent = Array.isArray(value) ? `Array(${value.length})` : "{\u2026}";
        details.append(summary, renderObject(value));
        return details;
      }
      const span = document.createElement("span");
      span.className = `debug-value debug-${typeof value}`;
      span.textContent = typeof value === "string" ? JSON.stringify(value) : String(value);
      return span;
    };
    const renderObject = (object, filter = false) => {
      const rows = document.createElement("dl");
      for (const [key, value] of Object.entries(object)) {
        if (filter && hidden.has(key)) {
          continue;
        }
        const dt = document.createElement("dt");
        dt.textContent = key;
        const dd = document.createElement("dd");
        dd.append(renderValue(value));
        rows.append(dt, dd);
      }
      return rows;
    };

    const show = () => {
      if (!last || !byLine) {
        return;
      }
      const { lineno, start, end } = last;
      const records = (byLine.get(lineno) || [])
        .filter(r => start === null || (r.start < end && start < r.end) || r.start === start);
      body.replaceChildren();
      const title = document.createElement("div");
      title.className = "debug-records-title";
      title.textContent = start === null
        ? `Line ${lineno}: ${records.length} records`
        : `Line ${lineno}, columns ${start}-${end}: ${records.length} records`;
      body.append(title);
      for (const { record } of records) {
        const details = document.createElement("details");
        details.open = true;
        details.className = "debug-record";
        const summary = document.createElement("summary");
        summary.textContent = [record.kind || (record.syntax !== undefined ? "source" : "record"),
                               record.pretty || record.sym || ""].join(" ").trim();
        details.append(summary, renderObject(record, true));
        body.append(details);
      }
    };

    propsButton.addEventListener("click", () => {
      props.hidden = !props.hidden;
      if (props.hidden || !byLine) {
        return;
      }
      const keys = new Set();
      for (const records of byLine.values()) {
        for (const { record } of records) {
          Object.keys(record).forEach(key => keys.add(key));
        }
      }
      props.replaceChildren(...[...keys].sort().map(key => {
        const label = document.createElement("label");
        const box = document.createElement("input");
        box.type = "checkbox";
        box.checked = !hidden.has(key);
        box.addEventListener("change", () => {
          box.checked ? hidden.delete(key) : hidden.add(key);
          localStorage.setItem(hiddenKey, [...hidden].join(","));
          show();
        });
        label.append(box, key);
        return label;
      }));
    });

    let scheduled = false;
    document.getElementById("content")?.addEventListener("mousemove", event => {
      if (scheduled || (typeof ContextMenu !== "undefined" && ContextMenu.active) ||
          !this.isExpanded() || li.closest("section")?.style.display === "none") {
        return;
      }
      const code = event.target.closest?.("code.source-line");
      if (!code) {
        return;
      }
      scheduled = true;
      requestAnimationFrame(async () => {
        scheduled = false;
        const row = code.closest(".source-line-with-number");
        const lineno = parseInt(row?.querySelector(".line-number")?.dataset.lineNumber ||
                                row?.id?.replace("line-", ""), 10);
        if (!lineno) {
          return;
        }
        // (The hovered token's columns: its text's offset in the line's.)
        let start = null;
        let end = null;
        const token = event.target.closest("[data-symbols]") ||
              (event.target !== code && code.contains(event.target) ? event.target : null);
        if (token) {
          const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
          let offset = 0;
          while (walker.nextNode()) {
            if (token.contains(walker.currentNode)) {
              break;
            }
            offset += walker.currentNode.length;
          }
          start = offset;
          end = offset + token.textContent.length;
        }
        if (last && last.lineno === lineno && last.start === start) {
          return;
        }
        last = { lineno, start, end };
        if (!byLine) {
          body.textContent = "Loading the records\u2026";
          await load();
        }
        show();
      });
    });

    return li;
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
      items.push(this.createRecordsWidget(link.href.replace(/#.*$/, "")));
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

      button.addEventListener("click", async () => {
        if (this.resultsJSONBox.hasAttribute("aria-hidden")) {
          this.resultsJSONBox.removeAttribute("aria-hidden");
          button.textContent = "Hide results JSON";
          // The page only has the results with `debug=true`, since they'd
          // double big pages; otherwise, they're the URL's JSON.
          let results = window.QUERY_RESULTS_JSON;
          if (results === undefined) {
            this.resultsJSONPre.textContent = "Loading...";
            try {
              const response = await fetch(document.location.href, {
                headers: { Accept: "application/json" },
              });
              results = await response.json();
            } catch (e) {
              results = `Couldn't load the results JSON: ${e}`;
            }
            if (this.resultsJSONBox.hasAttribute("aria-hidden")) {
              return;
            }
          }
          this.resultsJSONPre.textContent = JSON.stringify(results, undefined, 2);
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

  getEditorURL() {
    const status = this.canEditorOpen();
    if (!status.ok) {
      return null;
    }

    const { localRoot, sourcePath } = status;

    const absPath =
      `${localRoot.replace(/\/$/, "")}/${sourcePath}`;

    const line = this._resolveLineNumber();

    return this._expandTemplate(
      Settings.openInEditor.editorTemplate,
      { absPath, line }
    );
  }

  canEditorOpen() {
    if (!Settings.openInEditor?.enabled) {
      return { ok: false, reason: "Open in Editor is disabled" };
    }

    let treeMap;
    try {
      treeMap = JSON.parse(Settings.openInEditor.treePathMap || "{}");
    } catch {
      return { ok: false, reason: "Invalid tree → local path mapping JSON" };
    }

    const currentTree = Router.treeName;
    // 1. Check for an exact match first.
    let localRoot = treeMap[currentTree];

    // 2. If no exact match, look for a wildcard match.
    if (!localRoot) {
      for (const [key, path] of Object.entries(treeMap)) {
        if (key.includes("*")) {
          const pattern = key.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
          const regex = new RegExp(`^${pattern}$`);

          if (regex.test(currentTree)) {
            localRoot = path;
            break;
          }
        }
      }
    }

    if (!localRoot) {
      return {
        ok: false,
        reason:
          `No local path mapping for tree '${currentTree}'. Add a mapping for this tree in Settings → Open in Editor.`,
      };
    }

    return {
      ok: true,
      localRoot,
      sourcePath: Router.sourcePath,
    };
  }

  _resolveLineNumber() {
    if (Panel?.getLineNumberForSelectedSymbol) {
      const symLine = Panel.getLineNumberForSelectedSymbol();
      if (symLine !== undefined) {
        return symLine;
      }
    }

    if (window.Highlighter?.selectedLines?.size > 0) {
      return Math.min(...Highlighter.selectedLines);
    }

    return 1;
  }

  _expandTemplate(template, vars) {
    let out = template;
    for (const [k, v] of Object.entries(vars)) {
      out = out.replaceAll(`{{${k}}}`, v);
    }
    return out;
  }

  addOpenInEditorAction() {
    if (!Settings.openInEditor?.enabled) {
      return;
    }

    if (!Router.sourcePath) {
      return;
    }

    const h4 = document.createElement("h4");
    h4.textContent = "Editor";

    const ul = document.createElement("ul");

    const li = document.createElement("li");
    const button = document.createElement("button");

    button.classList.add("icon", "item");
    button.type = "button";
    button.textContent = "Open in Editor";
    button.dataset.accel = "\\";


    const accel = document.createElement("span");
    accel.classList.add("accel");
    accel.textContent = "\\";
    button.appendChild(accel);

    button.addEventListener("click", () => {
      const status = this.canEditorOpen();
      if (!status.ok) {
        return;
      }

      const url = this.getEditorURL();
      if (url) {
        window.location.href = url;
      }
    });

    const status = this.canEditorOpen();
    if (!status.ok) {
      button.disabled = true;
      button.title = status.reason;
    }

    li.appendChild(button);
    ul.appendChild(li);

    this.content.appendChild(h4);
    this.content.appendChild(ul);
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
    this.updateSelectedTokenIndex();
  }

  // Returns true if the event is dispatched inside the navigation panel.
  isOnPanel(event) {
    return !!event.target.closest("#panel");
  }

  prepareForSearch() {
    // Remove any item not shared between search and other contexts.
    for (const node of [...this.content.childNodes]) {
      this.content.removeChild(node);
    }

    if (this.isExpanded()) {
      this.toggle();
    }
  }

  maybeAutoCollapse() {
    const isFileView = !!document.getElementById("file");
    if (!isFileView) {
      return;
    }

    const widthThreshold =
      Settings.navPanel?.autoCollapseWidth ?? 1024;

    if (window.innerWidth < widthThreshold && this.isExpanded()) {
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
    // The default "dotted" style is hard to see.
    // the "dashed" style uses "5,2".
    if (path.getAttribute("stroke-dasharray") === "1,5") {
      path.setAttribute("stroke-dasharray", "2,3");
    }

    const dupe = path.cloneNode(false);
    dupe.classList.add("clicktarget");
    // Dashed/dotted edge should be clickable even in the gap part.
    dupe.removeAttribute("stroke-dasharray");
    // let's insert the clicktarget after the actual path so it is always what
    // the hit test finds.
    path.insertAdjacentElement("afterend", dupe);
  }
}
makeDiagramHoverEdges();
