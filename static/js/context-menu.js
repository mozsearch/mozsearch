function atUnescape(text) {
  return text.replace(/@([0-9A-F][0-9A-F])/g, (_, s) => String.fromCharCode(parseInt(s, 16)));
}

// The structured kinds of symbols whose values the "Follow" items follow.
const FOLLOWABLE_KINDS = new Set([
  "field", "variable", "arg", "parameter", "localVar", "property", "var",
]);
// IPDL files, whose structs' fields (`IPDL_` symbols) dataflow follows through
// the C++ IPDL generates for them (see tools/src/dataflow/trace.rs's
// `IpdlSlot`), and whose messages' parameters (`IPC_` symbols) it follows from
// the `Recv` implementations' parameters.
const IPDL_PATH_RE = /\.ipdlh?$/;

// C/C++ locals' and parameters' symbols (MozsearchIndexer.cpp's, and the
// tools' unified parameters'; see tools/src/file_format/local_symbols.rs).
function isCxxLocalSym(sym) {
  return /^V_[0-9a-fA-F]+_[0-9a-fA-F]+$/.test(sym);
}

// Whether a symbol is an IPDL struct, union or message, whose fields or
// parameters data flows through, so it has a data roll-up of its own (see
// `IpdlSlots` in tools/src/dataflow/summary.rs), unlike other IDL symbols,
// which are diagrammed by their language bindings.
function isIpdlDataSym(sym, info) {
  return /^IPDL_|^IPC_/.test(sym) && ["struct", "union", "ipc"].includes(info?.meta?.kind);
}

// The files whose languages dataflow understands (C++, JS/TS, Kotlin).
const DATAFLOW_PATH_RE = /\.(c|cc|cpp|cxx|h|hh|hpp|hxx|js|jsm|mjs|sjs|ts|tsx|jsx|kt|kts)$/;

// The URL of the search for `encodedQuery` (a URL-encoded `q`) on `tree`.
// Pages search with router.py's /search, whose links here shouldn't go to a
// search's only result, or, on trees searching with the pipeline-server's
// /query/default (the #data element's data-search; see `config::search_url`),
// there, in the context of the try push whose pages these are, if any
// (data-search-rev).  The code of another tree or revision on the page has
// its own (see `ContextMenu.searchContext`).
// A diagram's node's or edge's extra data (`kind` "nodes" or "edges"), from
// whichever of the page's graphs has it (see the tools' `cmd_graph`, whose
// graphs' ids are unique among them: ex: a block diagram's groups').
function graphExtra(kind, id) {
  if (typeof GRAPH_EXTRA === "undefined") {
    return undefined;
  }
  for (const extra of GRAPH_EXTRA) {
    const found = extra?.[kind]?.[id];
    if (found) {
      return found;
    }
  }
  return undefined;
}

function searchUrl(tree, encodedQuery) {
  const context = window.ContextMenu?.searchContext;
  const data = context?.getAttribute("data-tree") == tree && context.hasAttribute("data-search")
    ? context : document.getElementById("data");
  const search = data?.getAttribute("data-search");
  if (data?.getAttribute("data-tree") == tree && search?.endsWith("/query/default")) {
    const rev = data.getAttribute("data-search-rev");
    return `${search}?q=${encodedQuery}${rev ? `&rev=${encodeURIComponent(rev)}` : ""}`;
  }
  return `/${tree}/search?q=${encodedQuery}&redirect=false`;
}

// A link to the pipeline-server's /query/default (diagrams, following
// values), in the context of the try push whose code was clicked, or whose
// pages these are, if any (data-search-rev; see `searchUrl`).
function queryUrl(tree, queryString) {
  const context = window.ContextMenu?.searchContext;
  const data = context?.getAttribute("data-tree") == tree && context.hasAttribute("data-search")
    ? context : document.getElementById("data");
  const rev = data?.getAttribute("data-tree") == tree && data.getAttribute("data-search-rev");
  return `/${tree}/query/default?q=${encodeURIComponent(queryString)}` +
    (rev ? `&rev=${encodeURIComponent(rev)}` : "");
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
        if (item.needsSep) {
          li.classList.add("contextmenu-same-section-sep");
        } else {
          li.classList.add("contextmenu-same-section");
        }
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

class MenuItemWithSubMenu extends MenuItem {
  constructor(options) {
    super({
      ...options,
      menu: undefined,
      html: "",
      icon: "",
    });
    this.subMenuHTML = options.html;
    this.subMenuIcon = options.icon;
    this.menu = options.menu;
    this.subMenuInstance = null;
  }

  toKey() {
    return this.subMenuHTML;
  }

  isFocusable() {
    return true;
  }

  createListItem(...args) {
    let li = super.createListItem(...args);
    li.classList.add("has-submenu");
    li.addEventListener("mouseenter", () => {
      this.showSubMenu();
    });
    li.addEventListener("click", () => {
      this.toggleSubMenu();
    });
    li.addEventListener("mouseleave", () => {
      this.hideSubMenuSoon();
    });

    return li;
  }

  populateListItem(li, menu, pos) {
    super.populateListItem(li, menu, pos);

    const link = this.focusableElement;

    link.href = "#";

    const label = document.createElement("div");
    label.classList.add("submenu-label");
    if (this.subMenuIcon) {
      label.classList.add(`icon-${this.subMenuIcon}`);
    }
    label.innerHTML = this.subMenuHTML;

    const marker = document.createElement("div");
    marker.classList.add("submenu-marker");

    link.replaceChildren(label, marker);
  }

  showSubMenu() {
    if (!this.subMenuInstance) {
      this.subMenuInstance = new ContextSubMenu(this.tree, this.items, this.searches, this.menu, this);
    }

    this.subMenuInstance.open();
  }

  hideSubMenuSoon() {
    this.subMenuInstance?.hideSoon();
  }

  toggleSubMenu() {
    if (this.subMenuInstance?.isShown()) {
      this.subMenuInstance?.hide();
      return;
    }
    this.showSubMenu();
  }

  onKeyDown(event, menu, link, pos) {
    switch (event.key) {
      case "ArrowRight":
      case "Right": {
        this.showSubMenu();
        this.subMenuInstance.focusItemAt({ col: 0, row: 0 }, "start");
        return;
      }
    }

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

        ContextMenu.hide();
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
    this.href = searchUrl(this.tree, `symbol:${syms}`);
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
      needsSep: true,
      confidence: options.confidence,
    });

    this.pretty = options.pretty;
    this.sym = options.sym;
    this.tree = options.tree;
    this.isClass = options.isClass;
    this.showInheritance = options.showInheritance;
    // A subsystem's pseudo-symbol (see tools/src/file_format/rollup.rs),
    // which only block diagrams draw.
    this.isSubsystem = options.isSubsystem;
    this.isClassKind = options.isClassKind;
    // An IPDL struct, union or message, whose fields or parameters data
    // flows through, so it has a data roll-up but no calls' (see
    // `IpdlSlots` in tools/src/dataflow/summary.rs).
    this.isIpdl = options.isIpdl;
    // Whether diagrams of the symbol are by its symbol rather than its pretty
    // name, which other symbols may share (ex: a web spec's terms').
    this.bySym = options.bySym;

    this.links = [];

    this.pinnedPretty = localStorage.getItem(storageKey("diagram-pinned"));
    this.pinnedSym = localStorage.getItem(storageKey("diagram-pinned-sym"));
  }

  /**
   * A query term for a diagram (ex: "calls-to") of the symbol (or of the
   * pinned one), by its symbol (the term's "-sym" variant) if it's diagrammed
   * by symbol, else by its pretty name; quoted, with its quotes and
   * backslashes escaped.
   */
  diagramTerm(name, pinned = false) {
    const quote = value => `'${value.replace(/[\\']/g, c => `\\${c}`)}'`;
    const sym = pinned ? this.pinnedSym : (this.bySym ? this.sym : null);
    if (sym) {
      return `${name}-sym:${quote(sym)}`;
    }
    return `${name}:${quote(pinned ? this.pinnedPretty : this.pretty)}`;
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

  /**
   * Buttons for block diagrams (see tools/src/cmd_pipeline/cmd_block_diagram.rs)
   * of the class's or subsystem's calls and data, from crossref's roll-ups.
   */
  addBlockButtons(buttons, menu, pos) {
    const quote = value => `'${value.replace(/[\\']/g, c => `\\${c}`)}'`;
    // (A subsystem's block diagram is by its name, a class's by its symbol.)
    const focus = this.isSubsystem ? this.pretty : this.sym;
    const kinds = [
      ["Block", "", `Block diagram of ${this.pretty}'s calls`],
      ["Data", " block-edges:data", `Block diagram of ${this.pretty}'s data`],
    ];
    for (const [label, extra, title] of this.isIpdl ? kinds.slice(1) : kinds) {
      const link = document.createElement("a");
      link.classList.add("contextmenu-button");
      link.setAttribute("role", "menuitem");
      const queryString = `block-diagram:${quote(focus)}${extra}`;
      link.href = queryUrl(this.tree, queryString);
      link.addEventListener("keydown", event => {
        this.onKeyDown(event, menu, link, pos);
      });
      link.append(label);
      link.title = title;
      link.setAttribute("aria-label", title);
      buttons.append(link);
      this.links.push(link);
    }
  }

  populateListItem(li, menu, pos) {
    const withButtons = document.createElement("div");
    withButtons.classList.add("contextmenu-with-buttons");
    li.append(withButtons);

    const title = document.createElement("div");
    title.classList.add("contextmenu-section-title");
    title.classList.add(`icon-brush`);
    title.append(this.isIpdl ? `Diagram of ${this.pretty} (IPDL)` : "Diagram of " + this.pretty);
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

    if (this.isSubsystem || this.isIpdl) {
      this.addBlockButtons(buttons, menu, pos);
      return;
    }

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
      const queryString = `${this.diagramTerm("calls-to")} depth:4`;
      link.href = queryUrl(this.tree, queryString);
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
      const queryString = `${this.diagramTerm("calls-from")} depth:4`;
      link.href = queryUrl(this.tree, queryString);
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
        const queryString = `${this.diagramTerm("class-diagram")} depth:4`;
        link.href = queryUrl(this.tree, queryString);
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
        const queryString = `${this.diagramTerm("inheritance-diagram")} depth:4`;
        link.href = queryUrl(this.tree, queryString);
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
    if (this.isClass || this.isClassKind) {
      this.addBlockButtons(buttons, menu, pos);
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
        const queryString = `${this.diagramTerm("calls-between-source", true)} ${this.diagramTerm("calls-between-target")} depth:8`;
        link.href = queryUrl(this.tree, queryString);
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
        const queryString = `${this.diagramTerm("calls-between-source")} ${this.diagramTerm("calls-between-target", true)} depth:8`;
        link.href = queryUrl(this.tree, queryString);
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
        const queryString = `${this.diagramTerm("calls-between", true)} ${this.diagramTerm("calls-between")} depth:8`;
        link.href = queryUrl(this.tree, queryString);
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
    localStorage.setItem(storageKey("diagram-pinned"), this.pretty);
    if (this.bySym) {
      localStorage.setItem(storageKey("diagram-pinned-sym"), this.sym);
    } else {
      localStorage.removeItem(storageKey("diagram-pinned-sym"));
    }
  }
  removePinnedItem() {
    localStorage.removeItem(storageKey("diagram-pinned"));
    localStorage.removeItem(storageKey("diagram-pinned-sym"));
  }

  toKey() {
    return JSON.stringify({
      type: "diagram",
      pretty: this.pretty,
      // (Symbols diagrammed by symbol aren't others with their pretty name.)
      sym: this.bySym ? this.sym : null,
      tree: this.tree,
      isClass: this.isClass,
      showInheritance: this.showInheritance,
    });
  }
}

class DiagramOverloadMenuSection extends MenuItem {
  constructor(options) {
    super({
      section: "diagram-overload",
    });

    this.sym = options.sym;

    this.links = [];
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
    const overloadInfo = Diagram.overloadInfoPerSym.get(this.sym);
    for (const category of ["other", "depth"]) {
      const list = overloadInfo[category];

      if (list.length === 0) {
        continue;
      }

      const withButtons = document.createElement("div");
      withButtons.classList.add("contextmenu-with-buttons");
      li.append(withButtons);

      const title = document.createElement("div");
      title.classList.add("contextmenu-section-title");
      title.classList.add(`icon-brush`);
      if (category === "other") {
        title.append("This item hit the non-depth limit");
      } else {
        title.append("This item hit the depth limit");
      }
      withButtons.append(title);

      const titleButtons = document.createElement("div");
      title.classList.add("contextmenu-title-buttons");
      withButtons.append(titleButtons);
      {
        const link = document.createElement("a");
        link.classList.add("contextmenu-button");
        link.setAttribute("role", "menuitem");
        link.href = "#";
        link.addEventListener("keydown", event => {
          this.onKeyDown(event, menu, link, pos);
        });
        if (category === "other") {
          if (overloadInfo.canLift) {
            link.append("Lift the limit");
          } else {
            link.append("Cannot lift the limit");
            link.classList.add("disabled");
          }
        } else {
          if (Diagram.canIncreaseDepthLimit()) {
            link.append("Increase the depth");
          } else {
            link.append("Cannot increase the depth");
            link.classList.add("disabled");
          }
        }
        link.addEventListener("click", event => {
          event.preventDefault();

          if (category === "other") {
            if (overloadInfo.canLift) {
              Diagram.liftLimitFor(this.sym);
            }
          } else {
            if (Diagram.canIncreaseDepthLimit()) {
              Diagram.increaseDepthLimit();
            }
          }
          ContextMenu.hide();
        });
        titleButtons.append(link);

        this.links.push(link);
      }

      const addedLabels = new Set();

      const ul = document.createElement("ul");
      ul.classList.add("diagram-overload-list");
      li.append(ul);
      for (const overload of list) {
        const label = Diagram.getLabelForOverload(overload);
        if (addedLabels.has(label)) {
          continue;
        }
        addedLabels.add(label);

        const item = document.createElement("li");
        item.append(label);
        ul.append(item);
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

  toKey() {
    return JSON.stringify({
      type: "diagram-overload",
      sym: this.sym,
    });
  }
}

class ContextMenuOrSubMenu extends ContextMenuBase {
  constructor() {
    super();

    this.menu = document.createElement("ul");
    this.menu.classList.add("context-menu");
    this.menu.tabIndex = 0;
    this.menu.style.display = "none";
    this.menu.setAttribute("role", "menu");
    document.body.appendChild(this.menu);

    this.menu.addEventListener("mousedown", function (event) {
      // Prevent clicks on the menu to propagate
      // to the window, so that the menu is not
      // removed and links will be followed.
      event.stopPropagation();
    });

    this.openedSubMenus = new Set();
  }

  hide() {
    this.closeSubMenu();
    super.hide();
  }

  closeSubMenu() {
    for (const submenu of this.openedSubMenus) {
      submenu.hide();
    }
    this.openedSubMenus.clear();
  }

  convertSearchesToItems(searches, searchMenuItems, tree, confidence) {
    for (const { label, syms, def } of searches) {
      searchMenuItems.push(new SearchMenuItem({
        label,
        tree,
        syms,
        def,
        confidence,
      }));
    }
  }

  positionMenuAt(anchorLeft, anchorRight, anchorTop, anchorBottom){
    const x = anchorRight + window.scrollX;
    const y = anchorTop + window.scrollY;

    const viewportHeight = window.innerHeight;
    const spaceTowardsBottom = viewportHeight - anchorTop;
    const spaceTowardsTop = viewportHeight - spaceTowardsBottom;

    // Position the menu towards the bottom, and if that overflows and there's
    // more space to the top, flip it.
    this.menu.classList.remove("bottom");
    this.menu.style.bottom = "";
    this.menu.style.top = y + "px";
    this.menu.style.left = x + "px";
    this.menu.style.right = "";
    this.menu.style.maxHeight = "none";

    this.menu.style.display = "";
    this.menu.style.opacity = "0";
    this.menu.classList.remove("left");

    const rect = this.menu.getBoundingClientRect();

    const overflowsRight = rect.right > document.body.clientWidth;
    const spaceOnLeft = anchorLeft + window.scrollX;
    const fitsOnLeft = rect.width <= spaceOnLeft;

    if (overflowsRight && fitsOnLeft) {
      this.menu.style.left = "";
      this.menu.style.right = (document.body.clientWidth - spaceOnLeft) + "px";
      this.menu.classList.add("left");
    }

    // If it overflows, either flip it or constrain its height.
    if (rect.height > spaceTowardsBottom) {
      if (spaceTowardsTop > spaceTowardsBottom) {
        // Position it towards the top.
        this.menu.classList.add("bottom");
        const y2 = anchorBottom + window.scrollY;
        this.menu.style.bottom = viewportHeight - y2 + "px";
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
}

class ContextSubMenu extends ContextMenuOrSubMenu {
  constructor(tree, items, searches, parentMenu, parentItem) {
    super();

    this.menu.classList.add("context-submenu");

    this.menu.addEventListener("mouseenter", event => {
      this.keepShown();
    });
    this.menu.addEventListener("mouseleave", event => {
      this.hideSoon();
    });

    this.tree = tree;
    this.items = items;
    this.searches = searches;
    this.hideTimer = null;
    this.parentMenu = parentMenu;
    this.parentItem = parentItem;
  }

  isShown() {
    return this.menu.parentNode && this.menu.style.display !== "none";
  }

  keepShown() {
    if (this.hideTimer) {
      clearTimeout(this.hideTimer);
      this.hideTimer = null;
    }
  }

  open() {
    this.parentMenu.openedSubMenus.add(this);
    this.keepShown();

    document.body.appendChild(this.menu);

    const searchMenuItems = [];
    this.convertSearchesToItems(this.searches, searchMenuItems, this.tree, 1);

    const menuItems = [
      ...this.items,
      ...searchMenuItems,
    ];

    this.populateMenu(this.menu, menuItems);

    const openerRect = this.parentItem.focusableElement.getBoundingClientRect();
    this.positionMenuAt(openerRect.left, openerRect.right, openerRect.top, openerRect.bottom);
  }

  hideSoon() {
    this.hideTimer = setTimeout(() => {
      this.hide();
      this.hideTimer = null;
    }, 100);
  }

  hide() {
    this.menu.remove();
  }

  onKeyDown(event, item, itemPos) {
    switch (event.key) {
      case "ArrowLeft":
      case "Left":
        if (itemPos.col === 0) {
          this.parentMenu.focusItem(this.parentItem, "start");
          this.hide();
          return;
        }
    }

    super.onKeyDown(event, item, itemPos);
  }
}

// The names of the sites of symbols' documentation links (`docs` in their
// structured records, see `DocLink` in tools/src/file_format/analysis.rs).
const DOC_SITE_NAMES = {
  mdn: "MDN",
  android: "Android",
};

var ContextMenu = new (class ContextMenu extends ContextMenuOrSubMenu {
  constructor() {
    super();

    this.menu.id = "context-menu";

    this.selectedToken = null;

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
      // (A binding slot's spec definition, see BindingSlotLang::Spec.)
      if (lang === "spec") {
        return lang;
      }
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

    // A rendered spec's definition, or link to one, which has its symbol
    // (see the tools' `format::annotate_spec_symbols`): a link's click shows
    // the menu, which can follow it, rather than following it, but a click
    // with a modifier key (ex: to open it in a new tab) follows it.
    const renderedSpecToken = event.target.closest(".rendered-spec [data-symbols]");
    const renderedSpecLink = renderedSpecToken?.closest("a[href]");
    if (renderedSpecLink) {
      if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey || event.button !== 0) {
        return;
      }
      event.preventDefault();
    }

    // We expect to find symbols in:
    // - source listings ("code")
    // - diagrams ("svg")
    // - breadcrumbs ("breadcrumbs")
    // - rendered specs' definitions and links to them
    if (!event.target.closest("code") &&
        !renderedSpecToken &&
        !event.target.closest(".interactive-graph-block") &&
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

    // Breadcrumbs extra buttons don't need context menu.
    if (event.target.closest(".breadcrumbs-extra-button")) {
      return;
    }

    // Coverage graph svg doesn't need context menu
    if (event.target.closest("#coverage-graph")) {
      return;
    }

    // Breadcrumbs itself doesn't need context menu.
    if (event.target?.classList.contains("breadcrumbs")) {
      return;
    }

    // The click (especially, keyboard-initiated click) inside
    // a context menu shouldn't be handled here.
    if (event.target.closest(".context-menu")) {
      return;
    }

    // UI with its own menu for its elements (ex: the tokens in the
    // token-centric blame popup, see hyperblame.js) shows it with `showItems`.
    if (event.target.closest("[data-own-context-menu]")) {
      return;
    }

    // A link in a source listing (ex: a URL or a bug number in a comment, see
    // `links::linkify_comment`) shows the menu too, instead of leaving the
    // page, with an item to follow it, unless the click is for the link itself
    // (ex: with Ctrl, to open it in a new tab).
    const clickedLink = event.target.closest("a[href]");
    const sourceLink =
      clickedLink && event.target.closest("code.source-line")?.contains(clickedLink)
        ? clickedLink
        : null;
    if (sourceLink) {
      if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) {
        return;
      }
      event.preventDefault();
    }

    // (Another tree's code on the page, or another revision's, is in an
    // element saying whose it is, and how to search it, ex: an algorithm's
    // view's excerpts of the code citing it; see `format::citation_rows`.  And
    // a column of code from a different tree or revision than the others has
    // its own symbols' information, in SYM_INFOS.)
    this.searchContext = event.target.closest("[data-tree]");
    let tree = this.searchContext?.getAttribute("data-tree") ||
               document.getElementById("data").getAttribute("data-tree");
    const symInfoColumn = event.target.closest("[data-sym-info]")?.getAttribute("data-sym-info");
    const SYM_INFO = (symInfoColumn && window.SYM_INFOS?.[symInfoColumn]) || window.SYM_INFO;
    // (In a try push's context, whose symbols' jumps are to the push's head's
    // files, there; see `format::format_file_data`'s `search_rev`.)
    const searchRev = (this.searchContext || document.getElementById("data"))
      ?.getAttribute("data-search-rev");
    const sourceUrl = (jumpTree, jump) => searchRev && jumpTree === tree
      ? `/${tree}/rev/${searchRev}/${jump}`
      : `/${jumpTree}/source/${jump}`;

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
    // (A rendered spec's link, whose click shows this menu, can be followed;
    // see `renderedSpecLink` above.)
    if (renderedSpecLink) {
      jumpMenuItems.push(new GotoMenuItem({
        html: this.fmt("Follow the link to <strong>_</strong>", renderedSpecLink.textContent.trim()),
        href: renderedSpecLink.getAttribute("href"),
        icon: "export-alt",
        section: "jumps",
      }));
    }
    // then macro expansions
    let expansionMenuItems = [];
    let remainingExpansionMenuItems = [];
    // then searches
    let searchMenuItems = [];
    // then class field layout
    let fieldLayoutMenuItems = [];
    // then the text search
    let textSearchMenuItems = [];
    // then possible IDL and TS definitions
    let heuristicsMenuItems = [];
    // then glean submenu.
    let gleanMenuItems = [];
    // then sticky highlight option
    let stickyMenuItems = [];

    // then items for new/experimental features where
    // we don't want to mess with muscle memory at the top of the list.

    // diagram item
    let diagramMenuItems = [];
    let dataflowMenuItems = [];
    // then gc item
    let gcMenuItems = [];

    let idlSubMenuItems = [];
    let idlSubMenuSearches = [];

    let tsSubMenuItems = [];
    let tsSubMenuSearches = [];

    let gleanSymbol = null;

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

        // A block diagram's edge between classes: the calls between them,
        // and the data between their fields and methods (the first's field
        // diagram, with the second's; see the tools' `cmd_block_diagram`).
        // (An IPDL struct, union or message is a class for data, whose fields
        // or parameters data flows through.)
        const quote = value => `'${value.replace(/[\\']/g, c => `\\${c}`)}'`;
        const isClassInfo = info => ["class", "struct"].includes(info?.meta?.kind);
        const isDataClass = (sym, info) => isClassInfo(info) || isIpdlDataSym(sym, info);
        if (isClassInfo(srcSymInfo) && isClassInfo(targSymInfo) &&
            !isIpdlDataSym(srcSyms[0], srcSymInfo) && !isIpdlDataSym(targSyms[0], targSymInfo)) {
          const queryString = `calls-between-source:${quote(srcSymInfo.pretty)} ` +
            `calls-between-target:${quote(targSymInfo.pretty)} depth:6`;
          jumpMenuItems.push(new MenuItem({
            html: this.fmt("Calls between <strong>_</strong>", srcSymInfo.pretty) +
              this.fmt(" and <strong>_</strong>", targSymInfo.pretty),
            href: queryUrl(tree, queryString),
            icon: "brush",
            section: "jumps",
          }));
        }
        if (isDataClass(srcSyms[0], srcSymInfo) && isDataClass(targSyms[0], targSymInfo)) {
          jumpMenuItems.push(new MenuItem({
            html: this.fmt("Data between <strong>_</strong>", srcSymInfo.pretty) +
              this.fmt(" and <strong>_</strong>", targSymInfo.pretty),
            href: queryUrl(tree, `block-diagram:${quote(srcSyms[0])} block-level:field ` +
              `block-with:${quote(targSyms[0])} hier:pretty`),
            icon: "brush",
            section: "jumps",
          }));
        }
        // An edge between subsystems: their classes' calls, and data.
        const isSubsystem = sym => sym?.startsWith("SUBSYS_");
        if (isSubsystem(srcSyms[0]) && isSubsystem(targSyms[0])) {
          const name = sym => SYM_INFO[sym]?.pretty || sym.slice("SUBSYS_".length);
          for (const [label, edges] of [["Classes between", ""], ["Data between", " block-edges:data"]]) {
            jumpMenuItems.push(new MenuItem({
              html: this.fmt(`${label} <strong>_</strong>`, name(srcSyms[0])) +
                this.fmt(" and <strong>_</strong>", name(targSyms[0])),
              href: queryUrl(tree, `block-diagram:${quote(srcSyms[0])} block-level:class ` +
                `block-with:${quote(targSyms[0])}${edges}`),
              icon: "brush",
              section: "jumps",
            }));
          }
        }

        // Generate a "go to use"
        const edgeExtra = graphExtra("edges", symbolToken.id);
        if (edgeExtra?.jump && targSymInfo) {
          jumpMenuItems.push(new GotoMenuItem({
            html: this.fmt("Go to use of <strong>_</strong>", targSymInfo.pretty),
            // (A linked tree's edge's jump is to its files.)
            href: `/${edgeExtra.tree || tree}/source/${edgeExtra.jump}`,
            icon: "export-alt",
            section: "jumps",
          }));
        }
      }

      const findJSModuleGlobalDef = sym => {
        for (const def of document.querySelectorAll(`.syn_def[data-symbols*="${sym}"`)) {
          const syms = def.getAttribute("data-symbols").split(",");
          if (syms.includes(sym)) {
            return def;
          }
        }
        return null;
      };

      const getLinenoFor = elem => {
        const sourceline = elem.closest(`.source-line-with-number`);
        if (!sourceline) {
          return undefined;
        }

        const line = sourceline.querySelector(`.line-number`);
        if (!line) {
          return undefined;
        }

        return line.getAttribute("data-line-number");
      };

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
      let jsLocalSyms = [];
      let nonJSLocalSyms = [];
      // Symbols whose values the "Follow" items can follow: locals,
      // parameters, fields and globals.
      let followSyms = [];
      let sawDef = false;
      let hasModuleGlobalAndExported = false;
      symbols.forEach((sym, index) => {
        // Avoid processing the same symbol more than once.
        if (seenSyms.has(sym)) {
          return;
        }

        if (sym.startsWith("GLEAN_")) {
          gleanSymbol = sym;
        }

        if (sym.match(/^\d+-\d+$/)) {
          // JS local symbols from js-analyze.js.
          jsLocalSyms.push(sym);
          return;
        }
        if (isCxxLocalSym(sym)) {
          followSyms.push(sym);
          // C/C++ locals' and parameters' symbols from MozsearchIndexer.cpp
          // have crossref entries (see tools/src/file_format/local_symbols.rs),
          // so they have jumps and searches like other symbols, except in
          // indexes from before that, where their first occurrence in the
          // file is their declaration.
          if (!SYM_INFO[sym]?.pretty) {
            nonJSLocalSyms.push(sym);
            return;
          }
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
          // (scip's locals and parameters: "…/#N", "…().(name)".)
          if (sym.match(/^S_.+(\/#\d+|\(\)\.\([^)]+\))$/)) {
            followSyms.push(sym);
          }
          if (sym.match(/^S_(rs|java|jvm)_.+\/#\d+$/)) {
            // Rust/Java/Kotlin symbols from scip-indexer.rs.
            nonJSLocalSyms.push(sym);
            return;
          }
          // NOTE: Python also has S_py_ style symbol for local variables,
          //       but given it has no explicit declaration, it's hard to point
          //       the right thing.
          return;
        }

        let isJSModuleGlobal = false;
        if (sym.match(/^#M-.+$/)) {
          isJSModuleGlobal = true;
          if (!symInfo.jumps) {
            // This is a JS module global, which has multiple definitions across
            // files.
            // Within single file, all consumers should point single definition.
            const def = findJSModuleGlobalDef(sym);
            if (def) {
              const lineno = getLinenoFor(def);
              const localPath = document.location.pathname
                    .replace(/^\/[^\/]+\/source\//, "");
              symInfo.jumps = {
                def: `${localPath}#${lineno}`,
              };
            }
          }
        }

        const confidence = confidences[index];

        // (And an IPDL struct's field's accessor, for the field.)
        const slotOwner = symInfo.meta?.slotOwner;
        if (FOLLOWABLE_KINDS.has(symInfo.meta?.kind) ||
            (slotOwner?.sym?.startsWith("IPDL_") &&
             ["getter", "setter"].includes(slotOwner.slotKind))) {
          followSyms.push(sym);
        }

        // The symInfo is self-identifying via `pretty` and `sym` so we don't
        // need to try and include any extra context.
        exposeSymbolsForDebugging.push(symInfo);

        if (symInfo?.jumps?.idl === sourceLineClicked ||
            symInfo?.jumps?.def === sourceLineClicked ) {
          if (sym.startsWith("#") &&
              filteredSymTuples.length == 1 &&
              filteredSymTuples[0][0] == "#M-" + sym.slice(1)) {
            hasModuleGlobalAndExported = true;
          }
          if (!sawDef && !hasModuleGlobalAndExported) {
            // Transition to "kick out the implicit constructors" mode.
            sawDef = true;
            filteredSymTuples = [];
          }
          filteredSymTuples.push([sym, confidence, symInfo]);
        } else if (!sawDef) {
          filteredSymTuples.push([sym, confidence, symInfo]);
        }
      });

      // (Documentation links, once each, since a symbol and its binding slots,
      // ex: a WebIDL method and its spec's definition, can have the same.)
      const docsShown = new Set();
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
        //
        // (Nor locals' and parameters'.)
        // (But IPDL's structs and messages, for their data's block diagrams.)
        if (symInfo.meta && (symInfo.meta.implKind !== "idl" || isIpdlDataSym(sym, symInfo)) &&
            !isCxxLocalSym(sym)) {
          diagrammableSyms.push(symInfo);
        }

        // Define a helper we can also use for the binding slots below.
        const jumpify = (jumpref, pretty) => {
          // The symbol's documentation on other sites (ex: MDN's page for a
          // WebIDL method), from BCD; see tools/src/spec_analyze/bcd.rs.
          for (const doc of jumpref.meta?.docs || []) {
            if (docsShown.has(doc.url)) {
              continue;
            }
            docsShown.add(doc.url);
            jumpMenuItems.push(new MenuItem({
              html: this.fmt("_ documentation", DOC_SITE_NAMES[doc.site] || doc.site) +
                this.fmt(" for <strong>_</strong>", pretty),
              href: doc.url,
              icon: "docs",
              section: "jumps",
              classNames: ["contextmenu-docs"],
            }));
          }
          // A citation of an anchor its spec no longer has (so it has no
          // jumps): what removed it, and what it may be now, from the spec's
          // history (in the specs' tree; see tools/src/spec_analyze/stale.rs).
          const stale = jumpref.meta?.stale;
          if (stale) {
            jumpMenuItems.push(new MenuItem({
              html: this.fmt("<strong>_</strong> is no longer in its spec", pretty) +
                this.fmt(": removed by _", stale.removed.slice(0, 12)),
              href: `/${stale.tree}/commit/${stale.removed}`,
              icon: "help-circled",
              section: "jumps",
              classNames: ["contextmenu-stale-citation"],
            }));
            for (const suggestion of stale.suggestions || []) {
              jumpMenuItems.push(new MenuItem({
                html: this.fmt("Maybe it's now <strong>_</strong>", suggestion.pretty),
                href: `/${stale.tree}/source/${suggestion.jump}`,
                icon: "export-alt",
                section: "jumps",
                classNames: ["contextmenu-stale-suggestion"],
              }));
            }
          }
          if (!jumpref.jumps) {
            return;
          }
          // (A linked tree's symbol's jumps are to its files, ex: a web spec's
          // definition, from a Gecko comment citing it.)
          const jumpTree = jumpref.tree || tree;
          if (jumpref.jumps.idl && jumpref.jumps.idl !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to IDL definition of <strong>_</strong>", pretty),
              href: sourceUrl(jumpTree, jumpref.jumps.idl),
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }
          if (jumpref.jumps.glean && jumpref.jumps.glean !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to Glean definition of <strong>_</strong>", pretty),
              href: sourceUrl(jumpTree, jumpref.jumps.glean),
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }

          // (In a rendered spec, its definition in the rendered view, if it's
          // not the definition itself; see `renderedSpecToken` above.)
          const renderedDef = renderedSpecToken?.dataset.renderedDef;
          if (renderedSpecToken) {
            if (renderedDef) {
              jumpMenuItems.push(new GotoMenuItem({
                html: this.fmt("Go to definition of <strong>_</strong>", pretty),
                href: renderedDef,
                icon: "export-alt",
                section: "jumps",
                confidence,
              }));
            } else if (renderedSpecToken.id) {
              // (The definition itself, whose anchor the page's URL can have,
              // as a link to it: with the history API, rather than going to
              // it, which reloads nothing but scrolls; see search.js's
              // popstate handler.)
              const anchor = renderedSpecToken.id;
              jumpMenuItems.push(new MenuItem({
                html: this.fmt("Anchor to <strong>_</strong>", renderedSpecToken.textContent.trim() || pretty),
                action: () => {
                  history.pushState(history.state, "", `#${anchor}`);
                  ContextMenu.hide();
                },
                icon: "pin",
                section: "jumps",
                confidence,
              }));
            }
          } else if (jumpref.jumps.def && jumpref.jumps.def !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to definition of <strong>_</strong>", pretty),
              href: sourceUrl(jumpTree, jumpref.jumps.def),
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }

          // A citation of a web spec by an outdated URL (a moved or obsolete
          // spec's; see tools/src/spec_analyze/citations.rs) isn't an
          // up-to-date reference: say so, with the definition's current URL.
          if (symbolToken.classList.contains("syn_outdated") &&
              jumpref.meta?.implKind === "spec" && jumpref.meta.rendered) {
            jumpMenuItems.push(new MenuItem({
              html: this.fmt("This cites an outdated URL: <strong>_</strong>'s current one", pretty),
              href: jumpref.meta.rendered,
              icon: "help-circled",
              section: "jumps",
              classNames: ["contextmenu-outdated-citation"],
            }));
          }

          // A web spec's algorithm (or abstract operation, or IDL member with
          // steps, ex: a method's, which a Gecko WebIDL method's spec slot
          // leads to), beside the code citing it and its steps (in the tree
          // of its definition; see `format::format_algorithm`), or in a try
          // push's context, beside the code citing it at the push's head (see
          // `format::format_try_algorithm`).
          if (jumpref.jumps.def && jumpref.meta?.implKind === "spec" &&
              (["specAlgorithm", "specAbstractOp"].includes(jumpref.meta.kind) ||
               jumpref.meta.algorithm)) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Show <strong>_</strong> beside the code citing it", pretty),
              href: searchRev && jumpref.tree && jumpTree !== tree
                ? `/${tree}/rev-algorithm/${searchRev}/${jumpref.sym}`
                : `/${jumpTree}/algorithm/${jumpref.sym}`,
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
          }

          if (jumpref.jumps.decl && jumpref.jumps.decl !== sourceLineClicked) {
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to declaration of <strong>_</strong>", pretty),
              href: sourceUrl(jumpTree, jumpref.jumps.decl),
              icon: "export",
              section: "jumps",
              confidence,
            }));
          }

          // The symbol's documentation (a doc-def record), in its document's
          // rendered view if it's in a document.
          if (jumpref.jumps.doc && jumpref.jumps.doc !== sourceLineClicked) {
            const isDoc = /\.(md|markdown|rst)#/.test(jumpref.jumps.doc);
            jumpMenuItems.push(new GotoMenuItem({
              html: this.fmt("Go to documentation of <strong>_</strong>", pretty),
              href: `/${jumpTree}/${isDoc ? "rendered" : "source"}/${jumpref.jumps.doc}`,
              icon: "export-alt",
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
                    BlamePopup.triggerElement = expansionToken;
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
        //
        // Returns whether a menu entry was added or not.
        const directDefJumpify = (menu, jumpref, pretty) => {
          if (jumpref.jumps?.def && jumpref.jumps.def !== sourceLineClicked) {
            menu.push(new GotoMenuItem({
              html: this.fmt("Go to definition of <strong>_</strong>", pretty),
              href: sourceUrl(jumpref.tree || tree, jumpref.jumps.def),
              icon: "export-alt",
              section: "jumps",
              confidence,
            }));
            return true;
          }
          return false;
        }

        // Provide a submenu to jump to overrides.
        // If there's a single override, skip the submenu and just add the jump.
        // If there's more than MAX_OVERRIDEN_BY_LINKS, link to the complete list as well.
        const overrideJumpifyHelper = (jumpref) => {
          // Keep in sync with MAX_OVERRIDEN_BY_LINKS in `determine_desired_extra_syms_from_jumpref`
          // so that SYM_INFO has the data we want.
          const MAX_OVERRIDEN_BY_LINKS = 10;

          const overriddenBy = jumpref.meta?.overriddenBy;
          if (overriddenBy?.length) {
            if (overriddenBy.length === 1) {
              const overSym = overriddenBy[0];
              const overInfo = SYM_INFO[overSym];
              if (overInfo) {
                const overPretty = `Sole Override ${overInfo.pretty}`;
                directDefJumpify(jumpMenuItems, overInfo, overPretty);
              } else {
                console.warn(`Missing SYM_INFO for ${overSym}.`)
              }
            } else {
              const submenuItems = [];
              const submenuSearches = [];

              if (overriddenBy.length > MAX_OVERRIDEN_BY_LINKS) {
                submenuItems.push(new MenuItem({
                  html: `Show all (${MAX_OVERRIDEN_BY_LINKS} out of ${overriddenBy.length} listed below)`,
                  href: searchUrl(tree, `symbol:${encodeURIComponent(sym)}`),
                  section: "symbol-searches",
                  icon: "search",
                }));
              }

              for (const overSym of overriddenBy.slice(0, MAX_OVERRIDEN_BY_LINKS)) {
                const overInfo = SYM_INFO[overSym];
                if (overInfo) {
                  const hasDirectJump = directDefJumpify(submenuItems, overInfo, overInfo.pretty);
                  // When an override doesn't have a clear definition to jump to, provide a search entry instead.
                  // This happens for instance when the symbol is defined inside a macro because we list both the
                  // macro call and the token inside the macro definition as definition sites.
                  if (!hasDirectJump) {
                    submenuSearches.push({
                      label: overInfo.pretty,
                      syms: [overSym],
                    });
                  }
                } else {
                  console.warn(`Missing SYM_INFO for ${overSym}.`)
                }
              }

              jumpMenuItems.push(new MenuItemWithSubMenu({
                html: `${overriddenBy.length} Overrides`,
                tree,
                icon: "export-alt",
                section: "jumps",
                items: submenuItems,
                searches: submenuSearches,
                menu: this,
              }));
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

          if (slotOwner.sym.startsWith("GLEAN_")) {
            gleanSymbol = slotOwner.sym;
          }

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
                    directDefJumpify(jumpMenuItems, recvJumpref, `${implKind}${maybeLang} ${slot.slotKind}${maybeSlotImplKind} ${recvJumpref.pretty}`);
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
        } else if (hasModuleGlobalAndExported && sym.startsWith("#M-")) {
          jumpify(symInfo, symInfo.pretty);
          searches.push({
            label: "module global " + symInfo.pretty,
            syms: [sym],
            def: symInfo?.jumps?.def,
          });
        } else {
          jumpify(symInfo, symInfo.pretty);
          searches.push({
            label: symInfo.pretty,
            syms: [sym],
            def: symInfo?.jumps?.def,
          });
        }

        // Binding slots.  (An event's, ex: an observer topic's, are its
        // senders and receivers, which can be many, so we only link to them
        // if they're few, and they're not searches; see `AnalysisTarget::slot`
        // and `MAX_EVENT_SLOT_LINKS` in jumpref.rs.)
        const MAX_EVENT_SLOT_LINKS = 10;
        const isEvent = symInfo.meta?.kind === "event";
        if (symInfo.meta?.bindingSlots &&
            (!isEvent || symInfo.meta.bindingSlots.length <= MAX_EVENT_SLOT_LINKS)) {
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
            if (isEvent) {
              const role = slot.slotKind === "send" ? "sender" : "receiver";
              jumpify(slotJumpref, `${role} ${effectivePretty}`);
              continue;
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

        // Possible IDL/Glean definitions.
        if (symInfo.idl_syms) {
          for (const idlSym of symInfo.idl_syms) {
            const idlInfo = SYM_INFO[idlSym];
            if (!idlInfo) {
              continue;
            }
            let prefix = "";
            if (idlInfo?.meta?.bindingSlots) {
              for (const slot of idlInfo.meta.bindingSlots) {
                if (slot.sym === sym) {
                  prefix = `${slot.slotKind} `;
                }
              }
            }
            const def = idlInfo?.jumps?.idl;

            if (idlInfo?.meta?.implKind == "glean") {
              if (idlInfo.sym.startsWith("GLEAN_")) {
                gleanSymbol = idlInfo.sym;
              }

              // Glean uses qualified symbol, and it doesn't have to be
              // pushed into the submenu.
              searches.push({
                label: `Glean ${prefix}${idlInfo.pretty}`,
                syms: [idlInfo.sym],
                def,
              });
              continue;
            }

            if (def) {
              idlSubMenuItems.push(new GotoMenuItem({
                html: this.fmt(`Go to IDL definition of <strong>_</strong>`, idlInfo.pretty),
                href: sourceUrl(tree, def),
                icon: "export-alt",
                section: "jumps",
                confidence,
              }));
            }
            idlSubMenuSearches.push({
              label: `IDL ${prefix}${idlInfo.pretty}`,
              syms: [idlInfo.sym],
              def,
            });

            if (heuristicsMenuItems.length == 0) {
              heuristicsMenuItems.push(new MenuItemWithSubMenu({
                html: `Possible IDL definitions`,
                tree,
                icon: "export-alt",
                section: "heuristics",
                items: idlSubMenuItems,
                searches: idlSubMenuSearches,
                menu: this,
              }));
            }
          }
        }

        // If this is a js-analyze symbol, offer matching scip-typescript symbols as well
        if (symInfo.ts_syms && Array.isArray(symInfo.ts_syms)) {
          for (const tsSym of symInfo.ts_syms) {
            const tsInfo = SYM_INFO[tsSym];
            if (!tsInfo) {
              continue;
            }

            const def = tsInfo?.jumps?.def;
            if (def) {
              tsSubMenuItems.push(new GotoMenuItem({
                html: this.fmt(`Go to definition of <strong>_</strong>`, tsInfo.pretty),
                href: sourceUrl(tree, def),
                icon: "export-alt",
                section: "jumps",
                confidence,
              }));
            }

            const decl = tsInfo?.jumps?.decl;
            if (decl) {
              tsSubMenuItems.push(new GotoMenuItem({
                html: this.fmt(`Go to declaration of <strong>_</strong>`, tsInfo.pretty),
                href: sourceUrl(tree, decl),
                icon: "export-alt",
                section: "jumps",
                confidence,
              }));
            }

            tsSubMenuSearches.push({
              label: `${tsInfo.pretty}`,
              syms: [tsInfo.sym],
              def,
            });

            heuristicsMenuItems.push(new MenuItemWithSubMenu({
              html: `Possible TypeScript definitions`,
              tree,
              icon: "export-alt",
              section: "heuristics",
              items: tsSubMenuItems,
              searches: tsSubMenuSearches,
              menu: this,
            }));
          }
        }

        this.convertSearchesToItems(searches, searchMenuItems, tree, confidence);

        if (Settings.semanticInfo.enabled) {
          for (const jumpref of diagrammableSyms) {
            // (An IPDL struct's layout is its C++ class's, its slot's.)
            if ((jumpref?.meta?.kind === "class" || jumpref?.meta?.kind === "struct") &&
                !isIpdlDataSym(jumpref.sym, jumpref)) {
              let queryString = `field-layout:'${jumpref.pretty}'`;
              fieldLayoutMenuItems.push(new MenuItem({
                html: this.fmt("Class layout of <strong>_</strong>", jumpref.pretty),
                href: queryUrl(tree, queryString),
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
              isSubsystem: jumpref?.meta?.kind === "subsystem",
              isClassKind: ["class", "struct"].includes(jumpref?.meta?.kind),
              isIpdl: isIpdlDataSym(jumpref.sym, jumpref),
              showInheritance,
              // (A web spec's terms' pretty names are "<spec> <term>", which
              // its other definitions of the term, ex: a method's, share.)
              bySym: jumpref?.meta?.implKind === "spec",
              confidence,
            }));
          }

          if (Diagram.canIgnoreNode()) {
            diagramMenuItems.push(new MenuItem({
              html: "Ignore this node in the diagram",
              icon: "brush",
              section: "diagrams-ignore",
              action: () => {
                if (window.interactiveGraphInstance) {
                  window.interactiveGraphInstance.removeNodeByPrettyName(symInfo.pretty);
                  window.interactiveGraphInstance.updateQueryForIgnoredNode(symInfo.pretty);
                  this.hide();
                } else {
                  Diagram.ignoreNode(symInfo.pretty);
                }
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
                BlamePopup.triggerElement = symbolToken;
                BlameStripHoverHandler.keepVisible = true;
              },
            }));
          }
        }
      }

      const findJSLocalDef = sym => {
        // For JS local variables, the declaration has syn_deflocal class.
        for (const def of document.querySelectorAll(`.syn_deflocal[data-symbols*="${sym}"`)) {
          const syms = def.getAttribute("data-symbols").split(",");
          if (syms.includes(sym)) {
            return def;
          }
        }
        return null;
      };

      const findNonJSLocalDef = sym => {
        // For non-JS local variables, the first occurrence should be the
        // declaration.
        for (const def of document.querySelectorAll(`[data-symbols*="${sym}"`)) {
          const syms = def.getAttribute("data-symbols").split(",");
          if (syms.includes(sym)) {
            return def;
          }
        }
        return null;
      };

      const addLocalJump = def => {
        const lineno = getLinenoFor(def);
        if (!lineno) {
          return;
        }

        jumpMenuItems.push(new GotoMenuItem({
          html: this.fmt("Go to definition of <strong>_</strong>",
                         event.target.textContent),
          href: `${document.location.pathname}#${lineno}`,
          icon: "export-alt",
          section: "jumps",
        }));
      };

      for (const sym of jsLocalSyms) {
        if (event.target.classList.contains("syn_deflocal")) {
          continue;
        }

        const def = findJSLocalDef(sym);
        if (!def) {
          continue;
        }

        addLocalJump(def);
      }

      for (const sym of nonJSLocalSyms) {
        const def = findNonJSLocalDef(sym);
        if (!def || def === event.target) {
          continue;
        }

        addLocalJump(def);
      }

      // Follow the value forwards or backwards (in the languages dataflow
      // understands), from the line clicked.
      const isIpdl = IPDL_PATH_RE.test(Router.sourcePath || "");
      if (Settings.dataflow.enabled && sourceLineClicked &&
          (DATAFLOW_PATH_RE.test(Router.sourcePath) || isIpdl)) {
        for (const sym of new Set(followSyms)) {
          if (isIpdl && !sym.startsWith("IPDL_") && !sym.startsWith("IPC_")) {
            continue;
          }
          const at = `follow-at:${Diagram.quoteQueryValue(sourceLineClicked)}`;
          const quotedSym = Diagram.quoteQueryValue(sym);
          dataflowMenuItems.push(new MenuItem({
            html: this.fmt("Follow where <strong>_</strong> goes",
                           symbolToken.textContent),
            href: queryUrl(tree, `follow-forward:${quotedSym} ${at}`),
            icon: "right-dir",
            section: "dataflow",
          }));
          dataflowMenuItems.push(new MenuItem({
            html: this.fmt("Follow where <strong>_</strong> comes from",
                           symbolToken.textContent),
            href: queryUrl(tree, `follow-backward:${quotedSym} ${at}`),
            icon: "left-dir",
            section: "dataflow",
          }));
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
            BlamePopup.triggerElement = expansionToken;
            BlameStripHoverHandler.keepVisible = true;
          },
        }));
      }
    }

    let suppressSubstring = false;
    const overloadMenuItems = [];
    const overload = event.target.closest("[data-diagram-overload-symbol]");
    if (overload) {
      suppressSubstring = true;
      overloadMenuItems.push(new DiagramOverloadMenuSection({
        sym: overload.getAttribute("data-diagram-overload-symbol"),
      }));
    }

    if (!suppressSubstring) {
      // A word was clicked on, or a link, whose searches are for it instead.
      const words = sourceLink ? linkSearchTexts(sourceLink) : [getTargetWord()];
      for (const word of words.filter(Boolean)) {
        textSearchMenuItems.push(new MenuItem({
          html: this.fmt("Search for the substring <strong>_</strong>", word),
          href: searchUrl(tree, encodeURIComponent(word)),
          icon: "font",
          section: "text-searches",
          classNames: sourceLink ? ["contextmenu-url"] : undefined,
        }));
      }
    }

    if (gleanSymbol) {
      const gleanSubMenuItems = [];

      // The symbol uses the following format:
      //   * snake case
      //   * "." inside the key is replaced with "_"
      //   * keys are concatenated with "::"
      //
      // The symbol is generated also for extra_keys and its fields,
      // but all the menu below should use the probe itself.
      const parts = gleanSymbol.replace(/^GLEAN_/, "").split(/::/);
      const key0 = parts[0];
      const key1 = parts[1];

      const dictionaryPrefix = "https://dictionary.telemetry.mozilla.org/apps/firefox_desktop/metrics/";
      const glamPrefix = "https://glam.telemetry.mozilla.org/fog/probe/";
      const glamSuffix = "/explore?";

      // The Glean Dictionary and GLAM use the following format:
      //   * snake case
      //   * "." inside the key is replaced with "_"
      //   * keys are concatenated with "."
      const name = key0 + "_" + key1;

      function toCamelCase(s) {
        return s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
      }

      // The JS binding uses the following format:
      //   * camel case
      //   * "." is treated as "_" before applyingthe camel case
      //   * keys are concatenated with "."
      const testName = "Glean." + toCamelCase(key0) + "." + toCamelCase(key1)
            + ".testGetValue()";

      gleanSubMenuItems.push(new MenuItem({
        html: `Copy ${testName} for about:glean`,
        action: () => {
          navigator.clipboard.writeText(testName);
          this.hide();
        },
        icon: "docs",
        section: "glean",
      }));

      gleanSubMenuItems.push(new MenuItem({
        html: "Glean dictionary",
        href: dictionaryPrefix + name,
        icon: "search",
        section: "glean",
      }));

      gleanSubMenuItems.push(new MenuItem({
        html: "GLAM",
        href: glamPrefix + name + glamSuffix,
        icon: "search",
        section: "glean",
      }));

      gleanMenuItems.push(new MenuItemWithSubMenu({
        html: "Glean",
        tree,
        icon: "export-alt",
        section: "glean",
        items: gleanSubMenuItems,
        searches: [],
        menu: this,
      }));
    }

    let menuItems = [
      ...jumpMenuItems,
      ...expansionMenuItems,
      ...remainingExpansionMenuItems,
      ...searchMenuItems,
      ...fieldLayoutMenuItems,
      ...textSearchMenuItems,
      ...heuristicsMenuItems,
      ...gleanMenuItems,
      ...stickyMenuItems,
      ...dataflowMenuItems,
      ...diagramMenuItems,
      ...gcMenuItems,
      ...overloadMenuItems,
    ];

    // A link's first item follows it, unless another item already goes to
    // the same URL.
    if (sourceLink && !menuItems.some(item =>
      item.href && new URL(item.href, document.baseURI).href === sourceLink.href)) {
      menuItems.unshift(new MenuItem({
        html: this.fmt("Follow link to <strong>_</strong>", sourceLink.getAttribute("href")),
        href: sourceLink.href,
        icon: "export-alt",
        section: "link",
        classNames: ["contextmenu-url", "contextmenu-follow-link"],
        attrs: { title: sourceLink.href },
      }));
    }

    if (!menuItems.length) {
      return;
    }

    this.closeSubMenu();
    this.populateMenu(this.menu, menuItems);

    // (A link activated by the keyboard has no position: the menu goes below
    // it.)
    if (sourceLink && event.detail === 0 && !event.clientX && !event.clientY) {
      const rect = sourceLink.getBoundingClientRect();
      this.positionMenuAt(rect.left, rect.left, rect.bottom, rect.top);
    } else {
      this.positionMenuAt(event.clientX, event.clientX, event.clientY, event.clientY);
    }
  }

  /**
   * Show a menu of the given `MenuItem`s at a click's position, for UI with its
   * own menu (see `data-own-context-menu` in `tryShowOnClick`).
   */
  showItems(menuItems, event) {
    this.populateMenu(this.menu, menuItems);
    this.positionMenuAt(event.clientX, event.clientX, event.clientY, event.clientY);
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
    if (elem.tagName === "g" || elem.classList.contains("interactive-graph-block")) {
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
    if (typeof GRAPH_EXTRA === "undefined") {
      return;
    }

    // (All of the page's graphs', whose ids are unique among them.)
    const nodes = GRAPH_EXTRA.flatMap(extra => Object.entries(extra?.nodes ?? {}));
    for (const [node, nodeInfo] of nodes) {
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
      const edgeExtra = graphExtra("edges", id);
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

    let nodeExtra = graphExtra("nodes", id);
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

/**
 * The texts to search for for a link in a source listing: its text (ex: a
 * URL, as in the source), and a URL's without its query and fragment, if it
 * has them (ex: a spec's URL, for all its citations), or for a bug's link (see
 * `links::linkify_bug_numbers`), the bug's number.
 */
function linkSearchTexts(link) {
  const bug = link.href.match(/^https:\/\/bugzilla\.mozilla\.org\/show_bug\.cgi\?id=(\d+)$/);
  if (bug) {
    return [bug[1]];
  }
  const text = link.textContent.trim();
  const texts = [text];
  const base = text.replace(/[?#].*$/s, "");
  if (base !== text && /^[a-z][a-z0-9+.-]*:\/\/[^/]/i.test(base)) {
    texts.push(base);
  }
  return texts;
}

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

    const item = new MenuItem({
      html: "Top page",
      classNames: ["indent"],
      href: "/",
    });
    const columnBox = document.createElement("div");
    const list = document.createElement("ul");
    const li = item.createListItem(this, {col: columns.length, row: 0});
    list.append(li);
    columnBox.append(list);
    columns.push([item]);
    columnBoxes.push(columnBox);

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
