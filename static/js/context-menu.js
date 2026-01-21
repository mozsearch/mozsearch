function atUnescape(text) {
  return text.replace(/@([0-9A-F][0-9A-F])/g, (_, s) => String.fromCharCode(parseInt(s, 16)));
}

class ContextMenuBase {
  constructor() {
    this.menu = null;
    this.columns = [];

    window.addEventListener("mousedown", event => this.hideOnMouseDown(event));
    window.addEventListener("pageshow", event => this.hideOnPageShow(event));
  }

  hideOnMouseDown(event) {
    this.hide();
  }

  hideOnPageShow(event) {
    this.hide();
  }

  hide() {
    if (this.menu) {
      this.menu.style.display = "none";
    }
  }

  populateMenu(menu, menuItems) {
    const column = [];

    let mergedItems = new Map();
    for (const item of menuItems) {
      let key = item.toKey();
      if (mergedItems.has(key)) {
        mergedItems.get(key).merge(item);
        continue;
      }
      mergedItems.set(key, item);
    }

    menu.innerHTML = "";
    let lastSection = null;
    for (const item of mergedItems.values()) {
      const li = item.createListItem(this, {
        col: 0,
        row: column.length,
      });
      if (lastSection === null) {
        lastSection = item.section;
      } else if (lastSection === item.section) {
        // nothing to do for the same section
        li.classList.add("contextmenu-same-section");
      } else {
        li.classList.add("contextmenu-new-section");
        lastSection = item.section;
      }

      menu.appendChild(li);

      column.push(item);
    }

    // Default behavior for single column and no groups.
    // See TreeSwitcherMenu#setupMenu for multi-column + groups.
    this.columns = [column];
  }

  focusItemAt(pos, side) {
    while (!this.columns[pos.col][pos.row].isFocusable()) {
      pos.row++;
    }

    this.focusItem(this.columns[pos.col][pos.row], side);
  }

  focusItem(item, side) {
    this.focusElement(item.getFocusableElement(side));
  }

  focusElement(elem) {
    elem.focus();

    // Given focus needs user interaction, tell webtest separately.
    const event = new Event("focusmenuitem");
    event.targetItem = elem;
    document.dispatchEvent(event);
  }

  onKeyDown(event, item, itemPos) {
    const pos = { col: itemPos.col, row: itemPos.row };

    switch (event.key) {
      case "Esc":
      case "Escape":
        this.hide();
        event.preventDefault();
        return;
    }

    let side = "first";
    switch (event.key) {
      case "ArrowUp":
      case "Up":
        side = "last";
        pos.row--;
        if (pos.row >= 0 && !this.columns[pos.col][pos.row].isFocusable()) {
          // Skip label.
          pos.row--;
        }
        if (pos.row < 0) {
          if (pos.col > 0) {
            pos.col--;
            pos.row = this.columns[pos.col].length - 1;
          } else {
            pos.row = 0;
          }
        }
        break;

      case "ArrowDown":
      case "Down":
        pos.row++;
        if (pos.row >= this.columns[pos.col].length) {
          if (pos.col < this.columns.length - 1) {
            pos.col++;
            pos.row = 0;
          } else {
            pos.row = this.columns[pos.col].length - 1;
          }
        }
        break;

      case "Home":
        pos.row = 0;
        pos.col = 0;
        break;

      case "End":
        side = "last";
        pos.col = this.columns.length - 1;
        pos.row = this.columns[pos.col].length - 1;
        break;

      case "PageUp":
        pos.row = 0;
        break;

      case "PageDown":
        side = "last";
        pos.row = this.columns[pos.col].length - 1;
        break;

      case "ArrowLeft":
      case "Left":
        side = "last";
        pos.col--;
        if (pos.col < 0) {
          pos.col = 0;
        }
        if (pos.row >= this.columns[pos.col].length) {
          pos.row = this.columns[pos.col].length - 1;
        }
        break;

      case "ArrowRight":
      case "Right":
        pos.col++;
        if (pos.col >= this.columns.length) {
          pos.col = this.columns.length - 1;
        }
        if (pos.row >= this.columns[pos.col].length) {
          pos.row = this.columns[pos.col].length - 1;
        }
        break;

      default:
        return;
    }

    event.preventDefault();
    this.focusItemAt(pos, side);
  }
}

class MenuItem {
  constructor(options) {
    Object.assign(this, options);
    this.focusableElement = null;
  }

  toKey() {
    // By default, use all properties as key.
    return JSON.stringify(this);
  }

  merge(other) {
    // Given that key represents everything,
    // merge happens only when the item is fully equivalent.
    // Nothing to do here.
  }

  isFocusable() {
    return !!this.focusableElement;
  }

  getFocusableElement() {
    return this.focusableElement;
  }

  createListItem(menu, pos) {
    let li = document.createElement("li");
    li.classList.add("contextmenu-row");
    li.setAttribute("role", "none");

    if (this.confidence) {
      li.classList.add(`confidence-${this.confidence}`);
    }

    this.populateListItem(li, menu, pos);

    return li;
  }

  populateListItem(li, menu, pos) {
    let link = li.appendChild(document.createElement("a"));
    link.setAttribute("role", "menuitem");
    if (this.action) {
      link.addEventListener("click", (evt) => {
        evt.preventDefault();
        evt.stopPropagation();
        this.action();
      }, {
        // Debounce by only letting us hear one click.
        once: true
      });
      link.href = "#";
    } else if (this.href) {
      link.href = this.href;

      if (this.preaction) {
        link.addEventListener("click", (evt) => {
          this.preaction(evt);
        }, true);
      }
    }

    link.classList.add("contextmenu-link");
    if (this.icon) {
      link.classList.add(`icon-${this.icon}`);
    }
    if (this.classNames) {
      for (const name of this.classNames) {
        link.classList.add(name);
      }
    }
    if (this.attrs) {
      for (const [name, value] of Object.entries(this.attrs)) {
        link.setAttribute(name, value);
      }
    }
    link.addEventListener("keydown", event => {
      this.onKeyDown(event, menu, link, pos);
    });

    link.innerHTML = this.html;

    this.focusableElement = link;
  }

  onKeyDown(event, menu, link, pos) {
    menu.onKeyDown(event, this, pos);
  }
}

class GotoMenuItem extends MenuItem {
  constructor(options) {
    // Special handle a link to #lineno.
    if (options.href.startsWith(document.location.pathname + "#")) {
      const lineno = options.href.slice((document.location.pathname + "#").length);
      options.preaction = event => {
        if (event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) {
          return;
        }
        // See the popstate event handler in search.js
        Dxr.suppressNextPopState = Date.now();

        // The #lineno anchor doesn't exist by default.
        // Ensure the anchor exists at the point of navigation,
        // to avoid possible glitch.
        Highlighter.createSyntheticAnchor(lineno);
      };
    }

    super(options);
  }
}

class SearchMenuItem extends MenuItem {
  constructor(options) {
    super({
      html: ContextMenu.fmt("Search for <strong>_</strong>", options.label),
      href: "",
      section: "symbol-searches",
      icon: "search",
      confidence: options.confidence,
    });

    this.label = options.label;
    this.tree = options.tree;
    this.syms = options.syms;
    this.def = options.def;

    this.updateHref();
  }

  updateHref() {
    const syms = encodeURIComponent(this.syms.join(","));
    this.href = `/${this.tree}/search?q=symbol:${syms}&redirect=false`;
  }

  toKey() {
    if (this.def) {
      // This is mergeable.
      // Do not put syms in the key, and merge them later.
      return JSON.stringify({
        label: this.label,
        icon: this.icon,
        def: this.def,
        confidence: this.confidence,
      });
    }
    // Not mergeable.
    return super.toKey();
  }

  merge(other) {
    this.syms = [...new Set(this.syms.concat(other.syms))];
    this.updateHref();
  }
}

class DiagramMenuSection extends MenuItem {
  constructor(options) {
    super({
      icon: "brush",
      section: "callgraph",
      confidence: options.confidence,
    });

    this.pretty = options.pretty;
    this.sym = options.sym;
    this.tree = options.tree;
    this.isClass = options.isClass;
    this.showInheritance = options.showInheritance;

    this.links = [];

    this.pinnedPretty = localStorage.getItem("diagram-pinned");
  }

  isFocusable() {
    return true;
  }

  getFocusableElement(side) {
    if (side === "first") {
      return this.links[0];
    }
    return this.links.at(-1);
  }

  populateListItem(li, menu, pos) {
    const withButtons = document.createElement("div");
    withButtons.classList.add("contextmenu-with-buttons");
    li.append(withButtons);

    const title = document.createElement("div");
    title.classList.add("contextmenu-section-title");
    title.classList.add(`icon-brush`);
    title.append("Diagram of " + this.pretty);
    withButtons.append(title);

    const titleButtons = document.createElement("div");
    title.classList.add("contextmenu-title-buttons");
    withButtons.append(titleButtons);
    {
      const link = document.createElement("a");
      link.classList.add("contextmenu-button");
      link.classList.add("icon-pin");
      link.setAttribute("role", "menuitem");
      link.href = "#";
      link.addEventListener("keydown", event => {
        this.onKeyDown(event, menu, link, pos);
      });
      link.addEventListener("click", event => {
        event.preventDefault();

        this.pinItem();
        ContextMenu.hide();
      });
      link.title = `Pin ${this.pretty} for calls-between diagram`;
      link.setAttribute("aria-label", `Pin ${this.pretty} for calls-between diagram`);
      titleButtons.append(link);

      this.links.push(link);
    }

    const buttons = document.createElement("div");
    buttons.classList.add("contextmenu-buttons");
    li.append(buttons);

    {
      const link = document.createElement("a");
      link.classList.add("contextmenu-button");
      link.setAttribute("role", "menuitem");
      // TODO: Try dog-fooding with using the symbol-specific variant of this
      // whose query syntax is below.  The rationale for using pretty
      // identifiers is that they are more stable and more readable than
      // symbols.  It might be most practical to allow specializing a link
      // to just a single symbol from the page itself or in a sidebar
      // affordance, especially since it's hard to concisely express the
      // differences in signatures for overloads (although we have some
      // tentative plans to).
      // const queryString = `calls-to-sym:'${this.sym}' depth:4`;
      const queryString = `calls-to:'${this.pretty}' depth:4`;
      link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
      link.addEventListener("keydown", event => {
        this.onKeyDown(event, menu, link, pos);
      });
      link.append("Calls to");
      link.title = `Calls diagram to ${this.pretty}`;
      link.setAttribute("aria-label", `Calls diagram to ${this.pretty}`);
      buttons.append(link);

      this.links.push(link);
    }
    {
      const link = document.createElement("a");
      link.classList.add("contextmenu-button");
      link.setAttribute("role", "menuitem");
      const queryString = `calls-from:'${this.pretty}' depth:4`;
      link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
      link.addEventListener("keydown", event => {
        this.onKeyDown(event, menu, link, pos);
      });
      link.append("Calls from");
      link.title = `Calls diagram from ${this.pretty}`;
      link.setAttribute("aria-label", `Calls diagram from ${this.pretty}`);
      buttons.append(link);

      this.links.push(link);
    }
    {
      const link = document.createElement("a");
      link.classList.add("contextmenu-button");
      link.setAttribute("role", "menuitem");
      if (this.isClass) {
        const queryString = `class-diagram:'${this.pretty}' depth:4`;
        link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });
        this.links.push(link);
      } else {
        link.setAttribute("aria-disabled", "true");
        link.classList.add("disabled");
      }
      link.append("Class");
      link.title = `Class diagram of ${this.pretty}`;
      link.setAttribute("aria-label", `Class diagram of ${this.pretty}`);
      buttons.append(link);
    }
    {
      const link = document.createElement("a");
      link.classList.add("contextmenu-button");
      link.setAttribute("role", "menuitem");
      if (this.showInheritance) {
        const queryString = `inheritance-diagram:'${this.pretty}' depth:4`;
        link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });
        this.links.push(link);
      } else {
        link.setAttribute("aria-disabled", "true");
        link.classList.add("disabled");
      }
      link.append("Inheritance");
      link.title = `Inheritance diagram of ${this.pretty}`;
      link.setAttribute("aria-label", `Inheritance diagram of ${this.pretty}`);
      buttons.append(link);
    }

    if (this.pinnedPretty) {
      function createPinIcon() {
        const icon = document.createElement("span");
        icon.classList.add("icon-pin");
        icon.setAttribute("aria-label", "pinned");
        const hiddenText = document.createElement("span");
        hiddenText.classList.add("alt-for-icon");
        hiddenText.append("pinned");
        icon.append(hiddenText);
        return icon;
      }

      const withButtons = document.createElement("div");
      withButtons.classList.add("contextmenu-with-buttons");
      li.append(withButtons);

      const pinned = document.createElement("div");
      pinned.classList.add("contextmenu-subsection-title");
      pinned.append("with ", createPinIcon()," = ", this.pinnedPretty);
      withButtons.append(pinned);

      const titleButtons = document.createElement("div");
      title.classList.add("contextmenu-title-buttons");
      withButtons.append(titleButtons);
      {
        const link = document.createElement("a");
        link.classList.add("contextmenu-button");
        link.classList.add("icon-trash-empty");
        link.setAttribute("role", "menuitem");
        link.href = "#";
        link.addEventListener("click", event => {
          event.preventDefault();

          this.removePinnedItem();
          ContextMenu.hide();
        });
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });
        link.title = `Unpin ${this.pretty}`;
        link.setAttribute("aria-label", `Unpin ${this.pretty}`);
        titleButtons.append(link);

        this.links.push(link);
      }

      const buttons = document.createElement("div");
      buttons.classList.add("contextmenu-buttons");
      li.append(buttons);

      {
        const link = document.createElement("a");
        link.classList.add("contextmenu-button");
        link.setAttribute("role", "menuitem");
        const queryString = `calls-between-source:'${this.pinnedPretty}' calls-between-target:'${this.pretty}' depth:8`;
        link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });
        link.append("Calls from ", createPinIcon(), " to");
        link.title = `Calls diagram from ${this.pinnedPretty} to ${this.pretty}`;
        link.setAttribute("aria-label", `Calls diagram from ${this.pinnedPretty} to ${this.pretty}`);
        buttons.append(link);

        this.links.push(link);
      }

      {
        const link = document.createElement("a");
        link.classList.add("contextmenu-button");
        link.setAttribute("role", "menuitem");
        const queryString = `calls-between-source:'${this.pretty}' calls-between-target:'${this.pinnedPretty}' depth:8`;
        link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });

        link.append("Calls to ", createPinIcon(), " from");

        link.title = `Calls diagram from ${this.pretty} to ${this.pinnedPretty}`;
        link.setAttribute("aria-label", `Calls diagram from ${this.pretty} to ${this.pinnedPretty}`);
        buttons.append(link);

        this.links.push(link);
      }

      {
        const link = document.createElement("a");
        link.classList.add("contextmenu-button");
        link.setAttribute("role", "menuitem");
        link.setAttribute("role", "menuitem");
        const queryString = `calls-between:'${this.pinnedPretty}' calls-between:'${this.pretty}' depth:8`;
        link.href = `/${this.tree}/query/default?q=${encodeURIComponent(queryString)}`;
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });
        link.append("Calls between");
        link.title = `Calls diagram between ${this.pretty} and ${this.pinnedPretty}`;
        link.setAttribute("aria-label", `Calls diagram between ${this.pretty} and ${this.pinnedPretty}`);
        buttons.append(link);

        this.links.push(link);
      }
    }
  }

  onKeyDown(event, menu, link, pos) {
    let index = this.links.indexOf(link);
    if (index === -1) {
      menu.onKeyDown(event, this, pos);
      return;
    }

    switch (event.key) {
      case "ArrowUp":
      case "Up":
      case "ArrowLeft":
      case "Left":
        index--;
        break;

      case "ArrowDown":
      case "Down":
      case "ArrowRight":
      case "Right":
        index++;
        break;

      case "Home":
      case "PageUp":
        index = 0;
        break;

      case "End":
      case "PageDown":
        index = this.links.length - 1;
        break;

      default:
        menu.onKeyDown(event, this, pos);
        return;
    }

    if (index < 0 || index > this.links.length - 1) {
      menu.onKeyDown(event, this, pos);
      return;
    }

    event.preventDefault();
    menu.focusElement(this.links[index]);
  }

  pinItem() {
    localStorage.setItem("diagram-pinned", this.pretty);
  }
  removePinnedItem() {
    localStorage.removeItem("diagram-pinned");
  }

  toKey() {
    return JSON.stringify({
      type: "diagram",
      pretty: this.pretty,
      tree: this.tree,
      isClass: this.isClass,
      showInheritance: this.showInheritance,
    });
  }
}

var ContextMenu = new (class ContextMenu extends ContextMenuBase {
  constructor() {
    super();
    this.menu = document.createElement("ul");
    this.menu.className = this.menu.id = "context-menu";
    this.menu.tabIndex = 0;
    this.menu.style.display = "none";
    this.menu.setAttribute("role", "menu");
    document.body.appendChild(this.menu);

    this.selectedToken = null;

    this.menu.addEventListener("mousedown", function (event) {
      // Prevent clicks on the menu to propagate
      // to the window, so that the menu is not
      // removed and links will be followed.
      event.stopPropagation();
    });

    window.addEventListener("click", event => this.tryShowOnClick(event));
  }

  fmt(s, data) {
    data = data
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
    return s.replace("_", data);
  }

  fmtLang(lang) {
      lang = lang.toUpperCase();
      if (lang === "CPP") {
        lang = "C++";
      }
      return lang;
  }

  generatePseudoFileSymInfo(sym) {
    let path = atUnescape(sym.replace(/^(FILE|DIR)_/, ""));
    let pretty, def;
    if (sym.match(/^DIR_/)) {
      pretty = "directory " + path;
      def = path;
    } else {
      pretty = "file " + path;
      def = path + "#1";
    }
    return {
      sym: sym,
      pretty,
      jumps: {
        def,
      },
    };
  }

  sortBindingSlots(bindingSlots) {
    return bindingSlots.slice().sort((a, b) => {
      if (a.slotKind < b.slotKind) {
        return -1;
      }
      if (a.slotKind > b.slotKind) {
        return 1;
      }

      if (a?.implKind) {
        if (b?.implKind) {
          if (a.implKind < b.implKind) {
            return -1;
          }
          if (a.implKind > b.implKind) {
            return 1;
          }
        } else {
          return 1;
        }
      } else {
        if (b?.implKind) {
          return -1;
        }
      }

      return 0;
    });
  }

  tryShowOnClick(event) {
    if (Settings.fancyBar.enabled) {
      if (this.selectedToken) {
        if (!Panel?.isOnPanel?.(event)) {
          this.selectedToken.classList.remove("selected");
          this.selectedToken = null;
          Panel?.onSelectedTokenChanged?.();
        }
      }
    }

    // Don't display the context menu if there's a selection.
    // User could be trying to select something and the context menu will undo it.
    if (!window.getSelection().isCollapsed) {
      return;
    }

    // We expect to find symbols in:
    // - source listings ("code")
    // - diagrams ("svg")
    // - breadcrumbs ("breadcrumbs")
    if (!event.target.closest("code") &&
        !event.target.closest("svg") &&
        !event.target.closest(".breadcrumbs") &&
        !event.target.closest(".symbol-tree-table") &&
        !event.target.closest(".symbol")) {
      return;
    }

    // Tree switcher is inside breadcrumbs, but it has its own menu.
    if (event.target.closest("#tree-switcher") ||
        event.target.closest("#tree-switcher-menu")) {
      return;
    }

    // The click (especially, keyboard-initiated click) inside
    // a context menu shouldn't be handled here.
    if (event.target.closest(".context-menu")) {
      return;
    }

    let tree = document.getElementById("data").getAttribute("data-tree");

    // Figure out the source line this click was on, if it was on any line, so
    // that we can compare it against jump strings in order to avoid offering
    // the option to jump to the line the user literally just clicked on.
    let sourceLineClicked = null;
    {
      let sourceLineNode = event.target.closest("code");
      let lineNumberNode = sourceLineNode?.previousElementSibling;
      if (lineNumberNode && Router.sourcePath) {
        sourceLineClicked = `${Router.sourcePath}#${lineNumberNode.dataset.lineNumber}`;
      }
    }

    // jumps come first
    let jumpMenuItems = [];
    // then macro expansions
    let expansionMenuItems = [];
    let remainingExpansionMenuItems = [];
    // then searches
    let searchMenuItems = [];
    // then class field layout
    let fieldLayoutMenuItems = [];
    // then the text search
    let textSearchMenuItems = [];
    // then sticky highlight option
    let stickyMenuItems = [];

    // then items for new/experimental features where
    // we don't want to mess with muscle memory at the top of the list.

    // diagram item
    let diagramMenuItems = [];
    // then gc item
    let gcMenuItems = [];

    let expansions = {};
    let onlyOneExpansion = true;
    const expansionToken = event.target.closest("[data-expansions]");
    if (Settings.expansions.enabled) {
      if (expansionToken) {
        expansions = JSON.parse(expansionToken.dataset.expansions);
        onlyOneExpansion = Object.keys(expansions).length == 1;
        if (onlyOneExpansion) {
          for (const key in expansions) {
            onlyOneExpansion = Object.keys(expansions[key]).length == 1;
          }
        }
      }
    }

    let symbolToken = event.target.closest("[data-symbols]");
    if (symbolToken) {
      Hover.onSymbolClicked(symbolToken);
      
      if (Settings.fancyBar.enabled) {
        this.selectedToken = symbolToken;
        this.selectedToken.classList.add("selected");
        Panel?.onSelectedTokenChanged?.();
      }

      let symbols = symbolToken.getAttribute("data-symbols").split(",");
      let confidences = JSON.parse(symbolToken.getAttribute("data-confidences"));
      // if data-confidences is missing, assume everything is concrete
      if (!confidences) {
        confidences = Array(symbols.length);
        confidences.fill("concrete");
      }

      const seenSyms = new Set();
      // For debugging/investigation purposes, expose the symbols that got
      // clicked on on the window global.
      const exposeSymbolsForDebugging = window.CLICKED_SYMBOLS = [];

      // ## Diagram edge specialization
      if (symbolToken.id?.startsWith("Gide")) {
        // The "data-symbols" we have is of the form `A->B` where A is a comma
        // delimited list of the source symbols that were consolidated into a
        // single node, and the same deal with B.  This is exactly what was
        // declared to graphviz.  In acylic dot layouts, this edge will be
        // pointed downwards even if the arrowhead is visually pointing upwards
        // (ex: inheritance).
        const [srcSyms, targSyms] = symbolToken.getAttribute("data-symbols").split("->").map(x => x.split(","));

        // Just clear the normal symbol list as we don't actually want the
        // normal per-symbol behavior below.
        symbols = [];

        // We just want a pretty, so let's just use the first symbol of each.
        let srcSymInfo = SYM_INFO[srcSyms[0]];
        let targSymInfo = SYM_INFO[targSyms[0]];

        // Generate a "go to use"
        const edgeExtra = GRAPH_EXTRA[0].edges[symbolToken.id];
        if (edgeExtra.jump && targSymInfo) {
          jumpMenuItems.push(new GotoMenuItem({
            html: this.fmt("Go to use of <strong>_</strong>", targSymInfo.pretty),
            href: `/${tree}/source/${edgeExtra.jump}`,
            icon: "export-alt",
            section: "jumps",
          }));
        }
      }

      // ## First pass: Process symbols and potentially filter out implicit constructors
      //
      // In the future we can potentially use this pass to do more clever things,
      // but right now the main interesting situation that can arise is that the
      // user is clicking on a constructor where we have both the constructor
      // symbol plus all of the implicit constructors that will be invoked as
      // part of the constructor and we are weirdly attributing to the constructor.
      //
      // We can detect this case because we can detect when the user is clicking
      // on a line that's already the target of a definition jump.  And then in
      // that case we can filter out all the symbols that aren't definition jumps.
      //
      // In general, we only expect to see multiple symbols here when the symbol
      // varies per platform or as a result of implicit constructors like this.
      // Our logic to remove implicit constructors here will not affect the
      // platform case because all symbols will have the line as a definition.
      // (For other platforms where the definition is on a different line, the
      // symbol won't be present here because it won't have been mered in by the
      // merge-analyses step.)
      let filteredSymTuples = [];
      let sawDef = false;
      symbols.forEach((sym, index) => {
        // Avoid processing the same symbol more than once.
        if (seenSyms.has(sym)) {
          return;
        }

        let symInfo = SYM_INFO[sym];

        if (sym.match(/^(FILE|DIR)_/)) {
          if (!symInfo) {
            symInfo = this.generatePseudoFileSymInfo(sym);
          } else if (!symInfo.jumps) {
            symInfo = {
              ...this.generatePseudoFileSymInfo(sym),
              ...symInfo,
            };
          }
        }

        // XXX Ignore no_crossref data that's currently not useful/used.
        if (!symInfo || !symInfo.sym || !symInfo.pretty) {
          return;
        }

        const confidence = confidences[index];

        // The symInfo is self-identifying via `pretty` and `sym` so we don't
        // need to try and include any extra context.
        exposeSymbolsForDebugging.push(symInfo);

        if (symInfo?.jumps?.idl === sourceLineClicked ||
            symInfo?.jumps?.def === sourceLineClicked ) {
          if (!sawDef) {
            // Transition to "kick out the implicit constructors" mode.
            sawDef = true;
            filteredSymTuples = [];
          }
          filteredSymTuples.push([sym, confidence, symInfo]);
        } else if (!sawDef) {
          filteredSymTuples.push([sym, confidence, symInfo]);
        }
      });

      for (const [sym, confidence, symInfo] of filteredSymTuples) {
        let diagrammableSyms = [];
        // We need structured data to do diagramming; no structured data means
        // no diagramming.  Currently we expect this to be the case for our
        // JS analysis, but when we're able to switch at least some of the JS
        // analysis to scip-typescript that will change.
        //
        // That said, we want to ignore IDL symbols in favor of their language
        // binding symbols.  The main rationale right now is that for XPIDL
        // attributes we can't do anything for the pretty of the attribute, so
        // having a menu entry for it, especially given that we'll also have an
        // entry for the C++ getter (and potentially setter), is not helpful.
        //
        // Also, we don't currently de-duplicate the diagram links, but it
        // would be appropriate to do so or otherwise address that the traverse
        // logic itself will follow binding slots.
        if (symInfo.meta && symInfo.meta.implKind !== "idl") {
          diagrammableSyms.push(symInfo);
        }

        // Define a helper we can also use for the binding slots below.
        const jumpify = (jumpref, pretty) => {
          if (!jumpref.jumps) {
            return;
          }
          if (jumpref.jumps.idl && jumpref.jumps.idl !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to IDL definition of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.idl}`,
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }

          if (jumpref.jumps.def && jumpref.jumps.def !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to definition of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.def}`,
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }

          if (jumpref.jumps.decl && jumpref.jumps.decl !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to declaration of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.decl}`,
              icon: "export",
              section: "jumps",
              confidence,
            }));
          }

          for (const key in expansions) {
            if (key.startsWith(sym)) {
              for (const platform in expansions[key]) {
                const expansion = expansions[key][platform]
                let html;
                if (onlyOneExpansion) {
                  html = `Expansion: <code>${expansion}</code>`;
                } else {
                  html = `Expansion on ${platform}: <code>${expansion}</code>`;
                }
                expansionMenuItems.push(new MenuItem({
                  html: html,
                  classNames: ["contextmenu-expansion-preview"],
                  action: () => {
                    this.hide();
                    BlamePopup.expansionIndex = [key, platform, jumpref];
                    BlamePopup.blameElement = expansionToken;
                    BlameStripHoverHandler.keepVisible = true;
                  },
                  confidence,
                }));
              }
              delete expansions[key]
            }
          }
        }

        // Helper for cases like showing the recv def when the user is clicking
        // on a call to its send, but where we don't want to crowd the context
        // menu with the decl.
        const directDefJumpify = (jumpref, pretty) => {
          if (!jumpref.jumps) {
            return;
          }

          if (jumpref.jumps.def && jumpref.jumps.def !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to definition of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.def}`,
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }
        }

        // If the symbol has <= 2 overrides (we depend on the logic in our
        // rust `determine_desired_extra_syms_from_jumpref` helper at jumpref
        // generation time, so you can't just change the number here and have
        // things work out well), then emit direct def jump options.
        //
        // This is motivated by XPIDL where we want to be able to jump directly
        // to the overrides of the use of an XPIDL method in C++ where we are
        // dealing with an interface pointer, as well as for the binding slots
        // for when we are dealing with an XPIDL IDL def symbol.  This is
        // factored out into a helper because those are different call-sites; we
        // don't do an open-ended graph traversal.
        const overrideJumpifyHelper = (jumpref) => {
          if (jumpref.meta?.overriddenBy?.length && jumpref.meta?.overriddenBy?.length <= 2) {
            for (const overSym of jumpref.meta.overriddenBy) {
              const overInfo = SYM_INFO[overSym];
              if (overInfo) {
                let overPretty;
                if (jumpref.meta.overriddenBy.length === 1) {
                  overPretty = `Sole Override ${overInfo.pretty}`;
                } else {
                  overPretty = `Override ${overInfo.pretty}`;
                }
                directDefJumpify(overInfo, overPretty)
              }
            }
          }
        }

        let searches = [];

        // If we have a slotOwner, it can help make our "go to def" description
        // more descriptive and identical to what would be generated for when
        // the bindingSlot that refers to us from our slotOwner would describe.
        if (symInfo.meta?.slotOwner) {
          let slotOwner = symInfo.meta.slotOwner;
          let ownerJumpref = SYM_INFO[slotOwner.sym];
          // XXX Ignore no_crossref data that's currently not useful/used.
          if (ownerJumpref && ownerJumpref.sym && ownerJumpref.pretty) {
            let implKind = ownerJumpref.meta.implKind || "impl";
            if (implKind === "idl") {
              implKind = "IDL";
              // Our owner being an IDL type does not change whether this is
              // something diagrammable.
            }

            let maybeLang = "";
            if (slotOwner.slotLang) {
              maybeLang = ` ${this.fmtLang(slotOwner.slotLang)}`;
            }

            const canonLabel = `${implKind}${maybeLang} ${slotOwner.slotKind} ${symInfo.pretty}`;
            jumpify(symInfo, canonLabel);
            searches.push({
              label: canonLabel,
              syms: [sym],
              def: symInfo?.jumps?.def,
            });
            jumpify(ownerJumpref, ownerJumpref.pretty);

            // If our current symbol is an IPC Send method, offer a direct jump to the Recv def
            if (slotOwner?.slotKind === "send" && ownerJumpref) {
              const bindingSlots = this.sortBindingSlots(ownerJumpref?.meta?.bindingSlots);

              for (const slot of bindingSlots) {
                if (slot.slotKind === "recv") {
                  let recvJumpref = SYM_INFO[slot.sym];
                  if (recvJumpref?.pretty) {
                    let maybeSlotImplKind = "";
                    if (slot?.implKind) {
                      maybeSlotImplKind = ` ${slot.implKind}`;
                    }
                    directDefJumpify(recvJumpref, `${implKind}${maybeLang} ${slot.slotKind}${maybeSlotImplKind} ${recvJumpref.pretty}`);
                  }
                }
              }
            }
          } else {
            jumpify(symInfo, symInfo.pretty);
            searches.push({
              label: symInfo.pretty,
              syms: [sym],
              def: symInfo?.jumps?.def,
            });
          }
        } else {
          jumpify(symInfo, symInfo.pretty);
          searches.push({
            label: symInfo.pretty,
            syms: [sym],
            def: symInfo?.jumps?.def,
          });
        }

        // Binding slots
        if (symInfo.meta?.bindingSlots) {
          let implKind = symInfo.meta.implKind || "impl";
          if (implKind === "idl") {
            implKind = "IDL";
          }

          const bindingSlots = this.sortBindingSlots(symInfo.meta.bindingSlots);

          let allSearchSyms = [];
          for (const slot of bindingSlots) {
            // XXX Ignore no_crossref data that's currently not useful/used.
            let slotJumpref = SYM_INFO[slot.sym];
            // (we do handle the pretty not existing below)
            if (!slotJumpref || !slotJumpref.sym) {
              continue;
            }

            let maybeLang = "";
            if (slot.slotLang) {
              const lang = slot.slotLang;
              // Previously this was === "cpp", but the reality is that our
              // concern is that our JS analysis is soupy.  Especially with the
              // new TS XPIDL magic, if we can switch to scip-typescript at
              // least for system JS, we can remove this constraint.
              if (lang !== "js") {
                diagrammableSyms.push(slotJumpref);
              }
              maybeLang = ` ${this.fmtLang(lang)}`;
            }

            // Favor the slot's pretty if available.
            const effectivePretty = slotJumpref?.pretty || symInfo.pretty;
            let maybeSlotImplKind = "";
            if (slot?.implKind) {
              maybeSlotImplKind = ` ${slot.implKind}`;
            }
            let slotPretty =
              `${implKind}${maybeLang} ${slot.slotKind}${maybeSlotImplKind} ${effectivePretty}`;
            searches.push({
              label: slotPretty,
              syms: [slot.sym],
              def: symInfo?.jumps?.def,
            });
            allSearchSyms.push(slot.sym);

            if (slotJumpref) {
              jumpify(slotJumpref, slotPretty);
              // For XPIDL, we also want to do the same overriddenBy check here
              // that we do below so that if we're browsing the XPIDL source we
              // can go directly to the implementation rather than the pure
              // virtual decl that gets upgraded to a def.  (Unfortunately we
              // currently don't have a way to easily tell that that is what
              // happened to downgrade the def, although I guess we could
              // hardcode an assumption...)
              overrideJumpifyHelper(slotJumpref);
            }
          }

          // If there were multiple language bindings that we think might exist,
          // then generate a single roll-up search.
          if (allSearchSyms.length > 1 && implKind !== "StaticPrefs") {
            // Eat the default search if this was IDL, as currently the "search"
            // endpoint search for the synthetic symbol will only do upsells
            // which is not what people are used to.
            if (implKind === "IDL") {
              searches.shift();
              // Do put the synthetic symbol at the start of the explicit symbol
              // list, however.
              allSearchSyms.unshift(sym);
            }
            searches.push({
              label: `${implKind} ${symInfo.meta.kind} ${symInfo.pretty}`,
              syms: allSearchSyms,
              def: symInfo?.jumps?.def,
            });
          }
        }

        overrideJumpifyHelper(symInfo);

        // Possible IDL definitions.
        if (symInfo.idl_syms) {
          for (const idlSym of symInfo.idl_syms) {
            const idlInfo = SYM_INFO[idlSym];
            if (idlInfo) {
              let prefix = "";
              if (idlInfo?.meta?.bindingSlots) {
                for (const slot of idlInfo.meta.bindingSlots) {
                  if (slot.sym === sym) {
                    prefix = `${slot.slotKind} `;
                  }
                }
              }
              const def = idlInfo?.jumps?.idl;
              if (def) {
                jumpMenuItems.push(new GotoMenuItem({
                  html: this.fmt("Go to IDL definition of <strong>_</strong>", idlInfo.pretty),
                  href: `/${tree}/source/${def}`,
                  icon: "export-alt",
                  section: "jumps",
                  confidence,
                }));
              }
              searches.push({
                label: `IDL ${prefix}${idlInfo.pretty}`,
                syms: [idlInfo.sym],
                def,
              });
            }
          }
        }

        for (const { label, syms, def } of searches) {
          searchMenuItems.push(new SearchMenuItem({
            label,
            tree,
            syms,
            def,
            confidence,
          }));
        }

        if (Settings.semanticInfo.enabled) {
          for (const jumpref of diagrammableSyms) {
            if (jumpref?.meta?.kind === "class" || jumpref?.meta?.kind === "struct") {
              let queryString = `field-layout:'${jumpref.pretty}'`;
              fieldLayoutMenuItems.push(new MenuItem({
                html: this.fmt("Class layout of <strong>_</strong>", jumpref.pretty),
                href: `/${tree}/query/default?q=${encodeURIComponent(queryString)}`,
                // TODO: pick out a custom icon for this; "tasks" was great but
                // is already used for sticky highlight and so we now expect it
                // to have muscle memory implications so we can't repurpose it.
                icon: "docs",
                section: "layout",
                confidence,
              }));
            }
          }
        }

        if (Settings.diagramming.enabled) {
          for (const jumpref of diagrammableSyms) {
            let isClass = false;

            if ((jumpref?.meta?.kind === "class" || jumpref?.meta?.kind === "struct") &&
                jumpref?.meta?.fields?.length) {
              isClass = true;
            }

            let showInheritance = false;
            if (jumpref?.meta?.kind === "method" &&
                (jumpref?.meta?.overrides?.length || jumpref?.meta?.overriddenBy?.length)) {
              showInheritance = true;
            } else if (jumpref?.meta?.kind === "class" &&
                       (jumpref?.meta?.supers?.length || jumpref?.meta?.subclasses?.length)) {
              showInheritance = true;
            }

            diagramMenuItems.push(new DiagramMenuSection({
              pretty: jumpref.pretty,
              sym: jumpref.sym,
              tree,
              isClass,
              showInheritance,
              confidence,
            }));
          }

          if (Dxr.canIgnoreDiagramNode()) {
            diagramMenuItems.push(new MenuItem({
              html: "Ignore this node in the diagram",
              icon: "brush",
              section: "diagrams-ignore",
              action: () => {
                Dxr.ignoreDiagramNode(symInfo.pretty);
              },
            }));
          }
        }

        if (Settings.semanticInfo.enabled) {
          if (symInfo.meta && "canGC" in symInfo.meta) {
            gcMenuItems.push(new MenuItem({
              html: symInfo.meta.canGC ? "Can GC" : "Cannot GC",
              icon: "recycle",
              action: () => {
                this.hide();
                BlamePopup.blameElement = symbolToken;
                BlameStripHoverHandler.keepVisible = true;
              },
            }));
          }
        }
      }

      const tokenText = symbolToken.textContent;
      stickyMenuItems.push(new MenuItem({
        html: "Sticky highlight",
        action: () => { Hover.stickyHighlight(symbols, tokenText); },
        icon: "tasks",
        section: "highlights",
      }));
    }

    for (const key in expansions) {
      for (const platform in expansions[key]) {
        const expansion = expansions[key][platform];
        let html;
        if (onlyOneExpansion) {
          html = `Expansion: <code>${expansion}</code>`;
        } else {
          html = `Expansion on ${platform}: <code>${expansion}</code>`;
        }
        remainingExpansionMenuItems.push(new MenuItem({
          html: html,
          classNames: ["contextmenu-expansion-preview"],
          action: () => {
            this.hide();
            BlamePopup.expansionIndex = [key, platform];
            BlamePopup.blameElement = expansionToken;
            BlameStripHoverHandler.keepVisible = true;
          },
        }));
      }
    }

    let word = getTargetWord();
    if (word) {
      // A word was clicked on.
      textSearchMenuItems.push(new MenuItem({
        html: this.fmt("Search for the substring <strong>_</strong>", word),
        href: `/${tree}/search?q=${encodeURIComponent(word)}&redirect=false`,
        icon: "font",
        section: "text-searches",
      }));
    }

    let menuItems = [
      ...jumpMenuItems,
      ...expansionMenuItems,
      ...remainingExpansionMenuItems,
      ...searchMenuItems,
      ...fieldLayoutMenuItems,
      ...textSearchMenuItems,
      ...stickyMenuItems,
      ...diagramMenuItems,
      ...gcMenuItems,
    ];

    if (!menuItems.length) {
      return;
    }

    this.populateMenu(this.menu, menuItems);

    let x = event.clientX + window.scrollX;
    let y = event.clientY + window.scrollY;

    let viewportHeight = window.innerHeight;
    let spaceTowardsBottom = viewportHeight - event.clientY;
    let spaceTowardsTop = viewportHeight - spaceTowardsBottom;

    // Position the menu towards the bottom, and if that overflows and there's
    // more space to the top, flip it.
    this.menu.classList.remove("bottom");
    this.menu.style.bottom = "";
    this.menu.style.top = y + "px";
    this.menu.style.left = x + "px";
    this.menu.style.maxHeight = "none";

    this.menu.style.display = "";
    this.menu.style.opacity = "0";

    let rect = this.menu.getBoundingClientRect();
    // If it overflows, either flip it or constrain its height.
    if (rect.height > spaceTowardsBottom) {
      if (spaceTowardsTop > spaceTowardsBottom) {
        // Position it towards the top.
        this.menu.classList.add("bottom");
        this.menu.style.bottom = viewportHeight - y + "px";
        this.menu.style.top = "";
        if (rect.height > spaceTowardsTop) {
          this.menu.style.maxHeight = spaceTowardsTop + "px";
        }
      } else {
        // Constrain its height.
        this.menu.style.maxHeight = spaceTowardsBottom + "px";
      }
    }

    // Now the menu is correctly positioned, show it.
    this.menu.style.opacity = "";
    this.menu.focus();

    // Context menu doesn't focus on the item by default.
    this.menu.addEventListener("keydown", event => {
      if (event.target != this.menu) {
        return;
      }

      switch (event.key) {
        case "ArrowUp":
        case "Up": {
          const column = this.columns[0];
          this.focusItem(column[column.length - 1], "last");
          break;
        }
        case "ArrowDown":
        case "Down":
          const column = this.columns[0];
          this.focusItem(column[0], "first");
          break;
        default:
          return;
      }

      event.preventDefault();
    });
  }

  get active() {
    return this.menu.style.display != "none";
  }
})();

var Hover = new (class Hover {
  constructor() {
    this.items = [];
    this.graphItems = [];
    this.hoveredElem = null;
    this.sticky = false;
    window.addEventListener("mousedown", (evt) => {
      // Constrain de-highlighting to the primary mouse button; in particular,
      // scrolling via the middle mouse button should not disable the sticky
      // state.  Unfortunately I think scrolling with the primary mouse button
      // on the modern normally-hidden scrollbars, but I'm being conservative
      // with this change.
      if (this.sticky && evt.button === 0) {
        this.deactivateDiagram();
        this.deactivate();
      }
    });

    window.addEventListener("mousemove", event => this._handleMouseMove(event));
  }

  _handleMouseMove(event) {
    if (ContextMenu.active || this.sticky) {
      return;
    }

    let target = event.target;

    if (!(target instanceof Element)) {
      return;
    }

    let elem = target.closest("[data-symbols]");

    this._updateHoverState(elem);
  }

  _updateHoverState(elem) {
    // Don't recompute things if we're still hovering over the same element.
    if (elem === this.hoveredElem) {
      return;
    }
    if (!elem) {
      this.deactivateDiagram();
      this.deactivate();
      return;
    }

    let symbolNames = this.symbolsFromString(elem.getAttribute("data-symbols"));
    // We're hovering over a graph so we also want to hover related graph nodes.
    // We will still also potentially want to highlight any document spans as
    // well.
    if (elem.tagName === "g") {
      this.activateDiagram(elem);
    }

    this.activate(symbolNames, elem.textContent);
    this.hoveredElem = elem;
  }

  // Manually update highlight because mousemove is suppressed when menu is open.
  onSymbolClicked(elem) {
    this._updateHoverState(elem);
  }

  symbolsFromString(symbolStr) {
    if (!symbolStr || symbolStr == "?") {
      // XXX why the `?` special-case?
      return [];
    }
    return symbolStr.split(",");
  }

  deactivate() {
    for (let item of this.items) {
      item.classList.remove("hovered");
    }
    this.hoveredElem = null;
    this.items = [];
    this.sticky = false;
  }

  activate(symbols, visibleToken) {
    this.deactivate();
    this.items = this.findReferences(symbols, visibleToken);
    for (let item of this.items) {
      item.classList.add("hovered");
    }
  }

  findReferences(symbolNames, visibleToken) {
    if (!symbolNames.length) {
      return [];
    }

    let symbols = new Set(symbolNames);

    return [...document.querySelectorAll("span[data-symbols]")].filter(span => {
      // XXX The attribute check is cheaper, probably should be before.
      return (
        span.textContent == visibleToken &&
        this.symbolsFromString(span.getAttribute("data-symbols")).some(symbol =>
          symbols.has(symbol)
        )
      );
    });
  }

  #edgeReverseMap
  // Derive a map from edges to the source and target nodes by processing the
  // GRAPH_EXTRA node data on first use.  This could be generated on the server
  // but since the data is easily derived and we expect our graphs to be
  // O(1000), we don't expect this computation to be too bad.
  #ensureEdgeReverseMap() {
    if (this.#edgeReverseMap) {
      return;
    }

    this.#edgeReverseMap = new Map();
    if (!GRAPH_EXTRA?.[0]) {
      return;
    }

    for (const [node, nodeInfo] of Object.entries(GRAPH_EXTRA[0].nodes)) {
      for (const inEdge of nodeInfo.in_edges) {
        let edgeInfo = this.#edgeReverseMap.get(inEdge);
        if (!edgeInfo) {
          this.#edgeReverseMap.set(inEdge, [undefined, node]);
        } else {
          edgeInfo[1] = node;
        }
      }
      for (const outEdge of nodeInfo.out_edges) {
        let edgeInfo = this.#edgeReverseMap.get(outEdge);
        if (!edgeInfo) {
          this.#edgeReverseMap.set(outEdge, [node, undefined]);
        } else {
          edgeInfo[0] = node;
        }
      }
    }
  }

  activateDiagram(elem) {
    this.deactivateDiagram();

    let id;
    if (elem.id) {
      id = elem.id;
    } else {
      id = elem.parentElement.id;
    }
    if (id.startsWith("a_")) {
      id = id.substring(2);
    }

    const applyStyling = (targetId, clazzes) => {
      let maybeTarget = document.getElementById(targetId);
      // For the table rows, the id ends up on a "g" container with an "a_"
      // prefix.  We want to locate the a_ prefix and then adjust to its sole
      // child for consistency.
      if (!maybeTarget) {
        maybeTarget = document.getElementById(`a_${targetId}`);
        if (!maybeTarget) {
          return;
        }
        maybeTarget = maybeTarget.children[0];
      }
      maybeTarget.classList.add(...clazzes);

      this.graphItems.push([maybeTarget, clazzes])
    };

    // ## Hovered Edge
    if (id.startsWith("Gide")) {
      const edgeExtra = GRAPH_EXTRA[0].edges[id];
      if (!edgeExtra) {
        return;
      }

      this.#ensureEdgeReverseMap();

      const curEdgeHover = ["hovered-cur-edge"];
      elem.classList.add(...curEdgeHover);
      this.graphItems.push([elem, curEdgeHover]);

      let [srcNode, targNode] = this.#edgeReverseMap.get(id);

      const defaultInNodeHover = ["hovered-in-node"];
      applyStyling(srcNode, defaultInNodeHover);

      const defaultOutNodeHover = ["hovered-out-node"];
      applyStyling(targNode, defaultOutNodeHover);

      return;
    }

    let nodeExtra = GRAPH_EXTRA[0].nodes[id];
    if (!nodeExtra) {
      return;
    }

    // ## Hovered Node
    const curNodeHover = ["hovered-cur-node"];
    elem.classList.add(...curNodeHover);
    this.graphItems.push([elem, curNodeHover]);

    const defaultInNodeHover = ["hovered-in-node"];
    for (const [nid, clazzes] of nodeExtra.in_nodes) {
      applyStyling(nid, clazzes.length ? clazzes : defaultInNodeHover);
    }
    const defaultOutNodeHover = ["hovered-out-node"];
    for (const [nid, clazzes] of nodeExtra.out_nodes) {
      applyStyling(nid, clazzes.length ? clazzes : defaultOutNodeHover);
    }

    const inEdgeHover = ["hovered-in-edge"];
    for (const eid of nodeExtra.in_edges) {
      applyStyling(eid, inEdgeHover);
    }

    const outEdgeHover = ["hovered-out-edge"];
    for (const eid of nodeExtra.out_edges) {
      applyStyling(eid, outEdgeHover);
    }
  }

  deactivateDiagram() {
    for (const [item, clazzes] of this.graphItems) {
      item.classList.remove(...clazzes);
    }
    this.graphItems = [];
  }

  stickyHighlight(symbols, visibleToken) {
    ContextMenu.hide();
    this.activate(symbols, visibleToken);
    this.sticky = true;
  }
})();

function getTargetWord() {
  let selection = window.getSelection();
  if (!selection.isCollapsed) {
    return null;
  }

  let offset = selection.focusOffset;
  let node = selection.anchorNode;
  let string = node?.nodeValue;

  if (!string?.length) {
    return null;
  }

  function isWordChar(character) {
    // TODO: this could be more non-ascii friendly.
    //
    // Notable Changes:
    // - We have added "#" to deal with JS private symbols.  This will widen
    //   C preprocessor directives to include the leading #, which makes sense.
    //   This will also impact use of the "stringizing operator" for macros,
    //   where it won't be what we want.
    return /[#A-Z0-9_]/i.test(character);
  }

  if (offset < string.length && !isWordChar(string[offset])) {
    // Not really in a word.
    return null;
  }

  let start = offset;
  let end = offset;

  while (start > 0 && isWordChar(string[start - 1])) {
    --start;
  }
  while (end < string.length && isWordChar(string[end])) {
    ++end;
  }

  if (end <= start) {
    return null;
  }

  return string.substring(start, end);
}

var TreeSwitcherMenu = new (class TreeSwitcherMenu extends ContextMenuBase {
  constructor() {
    super();
    this.button = document.getElementById("tree-switcher");
    this.menu = document.getElementById("tree-switcher-menu");

    if (!this.button || !this.menu) {
      return;
    }

    this.button.addEventListener("click", () => {
      if (this.isShown()) {
        this.hide();
      } else {
        this.setupMenu();
        this.show();
        this.focusCurrentTree();
      }
    });
  }

  show() {
    this.menu.style.display = "flex";
    this.button.setAttribute("aria-expanded", "true");
  }

  isShown() {
    return this.menu.style.display == "flex";
  }

  hide() {
    super.hide();
    this.button.setAttribute("aria-expanded", "false");
  }

  hideOnMouseDown(event) {
    if (event.target.closest("#tree-switcher-menu")) {
      return;
    }
    if (event.target.closest("#tree-switcher")) {
      return;
    }

    this.hide();
  }

  setupMenu() {
    const columns = [];
    const columnBoxes = [];

    for (const groups of TREE_LIST) {
      const columnBox = document.createElement("div");
      const column = [];
      for (const group of groups) {
        const menuItems = [];

        const groupIdPart = group.name.toLowerCase().replace(/[^a-z0-9]/g, "-");
        const groupId = "tree-switcher-group-" + groupIdPart;
        const groupListId = "tree-switcher-group-list-" + groupIdPart;

        const label = document.createElement("label");
        label.id = groupId;
        label.setAttribute("for", groupListId);
        label.classList.add("context-menu-group-label");
        label.textContent = group.name;
        column.push({
          isFocusable: () => false,
        });
        columnBox.append(label);

        const list = document.createElement("ul");
        list.id = groupListId;
        list.setAttribute("role", "group");
        for (const rawItem of group.items) {
          const label = rawItem.label ? rawItem.label : rawItem.value;
          const tree = rawItem.value;

          const item = new MenuItem({
            html: label,
            classNames: ["indent"],
            href: document.location.pathname.replace(/^\/[^\/]+\//, `/${tree}/`)
              + document.location.search
              + document.location.hash,
            attrs: {
              "data-tree": tree,
            },
          });

          const li = item.createListItem(this, {
            col: columns.length,
            row: column.length,
          });

          li.setAttribute("aria-labelledby", groupId);

          list.append(li);
          column.push(item);
        }
        columnBox.append(list);
      }
      columns.push(column);
      columnBoxes.push(columnBox);
    }

    this.columns = columns;

    this.menu.replaceChildren(...columnBoxes);
  }

  getCurrentTree() {
    const m = document.location.pathname.match(/^\/([^\/]+)\//);
    if (m) {
      return m[1];
    }

    // Fallback
    return "mozilla-central";
  }

  focusCurrentTree() {
    const tree = this.getCurrentTree();
    const elem = this.menu.querySelector(`a[data-tree="${tree}"]`);
    if (!elem) {
      this.menu.focus();
    }

    this.focusElement(elem);
  }
});
