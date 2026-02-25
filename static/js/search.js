var Dxr = new (class Dxr {
  constructor() {
    let constants = document.getElementById("data");

    // This will usually be "/"
    this.wwwRoot = constants.getAttribute("data-root");
    // This will look like "firefox-main"
    this.tree = constants.getAttribute("data-tree");
    this.baseUrl = location.protocol + "//" + location.host;
    // This will end up "/TREE/static/icons/"
    this.icons = this.wwwRoot + `${this.tree}/static/icons/`;
    // This will usually be "/TREE/search"
    this.searchUrl = constants.getAttribute("data-search");
    this.timeouts = {
      search: 300,
      // We start the history timeout after the search updates (i.e., after
      // timeouts.search has elapsed).
      history: 2000 - 300,
    };
    this.searchBox = document.getElementById("search-box");

    // TODO(emilio): Maybe these should be their own web component or
    // something.
    this.fields = {
      query: document.getElementById("query"),
      path: document.getElementById("path"),
      caseSensitive: document.getElementById("case"),
      regexp: document.getElementById("regexp"),
    };
    this.bubbles = {
      query: document.getElementById("query-bubble"),
      path: document.getElementById("path-bubble"),
    };

    this.setupColSelector();

    this.setupClassSorter();

    this.startSearchTimer = null;
    // The timer to move to the next url.
    this.historyTimer = null;
    // The controller to allow aborting a fetch().
    this.fetchController = null;

    window.addEventListener("pageshow", () =>
      this.initFormFromLocalStorageOrUrl()
    );

    // We use pushState in Dxr.updateHistory etc.
    // If an user initiates the browser's back/forward button,
    // they triggers popstate events without the actual navigation.
    //
    // Let's take the following scenario:
    //
    //  1. the user navigates to foo.cpp file,
    //  2. the user types a text into the search field
    //  3. updateHistory pushes the state
    //  4. the user hits the back button
    //
    // At the step 3, the location becomes search?q=...,
    // and at the step 4, the location becomes the foo.cpp file,
    // but the page content remains with the search result.
    //
    // In order to mitigate the issue with simple approach,
    // we perform reload.
    window.addEventListener("popstate", event => {
      // One exception where we don't want to perform reload is the
      // "Go to ..." menu item, which can navigate to #lineno in the
      // same document.
      // Navigation to a document fragment also triggers a popstate event.
      //
      // We detect it by the suppressNextPopState property, which is set
      // by the "Go to ..." menu item's event handler.
      //
      // The timestamp is used to avoid getting confused by
      // unrelated click events on the menu item.
      const suppressNextPopState = this.suppressNextPopState;
      this.suppressNextPopState = 0;
      if (suppressNextPopState && suppressNextPopState > Date.now() - 1000) {
        ContextMenu.hide();
        return;
      }
      window.location.reload();
    });

    // XXX hacky mechanism so that we only run this on pages with a "search"
    // header rather than a "query" header.  We should refactor this and the
    // general code listing generation so that we:
    // - pick search versus query based on Setting and can change by user
    //   choice even on any page.
    // - explicitly understand if it's operating in search or query mode.
    if (this.fields.path) {
      this.fields.query.addEventListener("input", () => this.startSearchSoon());
      this.fields.path.addEventListener("input", () => this.startSearchSoon());
      this.fields.regexp.addEventListener("change", () => this.startSearch());
      this.fields.caseSensitive.addEventListener("change", event => {
        window.localStorage.setItem("caseSensitive", event.target.checked);
        this.startSearch();
      });
      this.initFormFromLocalStorageOrUrl();
    }

    // Set to the current timestamp when we want to suppress the next
    // popstate event handler.
    // See the popstate event handler for more details.
    this.suppressNextPopState = 0;
  }

  cancel(cancelFetch = true) {
    if (this.startSearchTimer) {
      clearTimeout(this.startSearchTimer);
      this.startSearchTimer = null;
    }
    if (this.historyTimer) {
      clearTimeout(this.historyTimer);
      this.historyTimer = null;
    }
    if (cancelFetch && this.fetchController) {
      this.fetchController.abort();
      this.fetchController = null;
    }
  }

  startSearchSoon() {
    this.cancel(/* cancelFetch = */ false);
    this.startSearchTimer = setTimeout(() => {
      this.startSearchTimer = null;
      this.startSearch();
    }, this.timeouts.search);
  }

  async startSearch() {
    this.cancel();

    let query = this.fields.query.value;
    let path = this.fields.path.value.trim();

    if (!query.length && !path.length) {
      return this.hideBubbles();
    }

    if (query.length < 3 && path.length < 3) {
      return this.showBubble(
        "info",
        "Enter at least 3 characters to do a search.",
        query.length ? "query" : "path"
      );
    }

    this.hideBubbles();

    let url = this.constructURL();

    let controller = new AbortController();

    this.fetchController = controller;

    this.searchBox.classList.add("in-progress");
    let results;
    try {
      let response = await fetch(url.href, {
        headers: {
          Accept: "application/json",
        },
        signal: this.fetchController.signal,
      });
      results = await response.json();
      if (!response.ok) {
        return this.showBubble(results.error_level, results.error_html);
      }
    } catch (error) {
      if (controller.signal.aborted) {
        // This fetch was cancelled in order to do a new query, nothing to do
        // here.
        return;
      }
      return this.showBubble("error", "An error occurred. Please try again.");
    } finally {
      this.searchBox.classList.remove("in-progress");
    }

    populateResults(results, false, false);

    this.updateHistory(url);
  }

  constructURL() {
    let url = new URL(this.searchUrl, window.location);
    url.searchParams.set("q", this.fields.query.value);
    if (this.fields.path) {
      url.searchParams.set("path", this.fields.path.value.trim());
    }
    if (this.fields.caseSensitive) {
      url.searchParams.set("case", this.fields.caseSensitive.checked);
    }
    if (this.fields.regexp) {
      url.searchParams.set("regexp", this.fields.regexp.checked);
    }
    return url;
  }

  updateHistory(url) {
    this.historyTimer = setTimeout(() => {
      this.historyTimer = null;
      window.history.pushState({}, "", url.href);

      Panel.updateDebugSectionForLocation();
    }, this.timeouts.history);
  }

  initFormFromLocalStorageOrUrl() {
    // XXX similar to in the constructor, we're using the path field as an
    // indication of the mode we're in, but we should clean this up.
    if (!this.fields.path) {
      return;
    }

    let url = new URL(location.href);
    let params = url.searchParams;

    // If the `case` param is in the URL, use its boolean value, so that the
    // checkbox reflects what's in the URL. If the `case` param is not in the
    // URL and this is in fact a search URL, then the search is implicitly
    // case-insensitive, so ensure the checkbox reflects that. Don't update
    // the localStorage value in either of these cases, because the user
    // may just have received a link from somebody else and we don't want to
    // update the user's saved defaults. The saved defaults *only* get updated
    // when the user explicitly clicks on the checkbox and the change event
    // listener triggers.
    // Finally, if we're not in a search already, have the checkbox reflect
    // the saved default from localStorage.
    let caseSensitive = params.get("case");
    if (caseSensitive) {
      caseSensitive = caseSensitive === "true";
    } else if (params.get("q")) {
      caseSensitive = false;
    } else {
      caseSensitive = window.localStorage.getItem("caseSensitive") === "true";
    }
    this.fields.caseSensitive.checked = caseSensitive;

    this.fields.regexp.checked = params.get("regexp") === "true";

    let query = params.get("q");
    if (query) {
      this.fields.query.value = query;
    }

    let path = params.get("path");
    if (path) {
      this.fields.path.value = path;
    }
  }

  // Hang an advisory message off the search field.
  // @param {string} level - The seriousness: 'info', 'warning', or 'error'
  // @param {string} html - The HTML message to be displayed
  // @param {string} which - 'query' or 'path', defaults to the focused element
  // or 'query' otherwise.
  showBubble(level, html, which) {
    if (!which) {
      which = document.activeElement == this.fields.path ? "path" : "query";
    }
    let other = which == "path" ? "query" : "path";
    this.hideBubble(other);

    let bubble = this.bubbles[which];
    bubble.classList.remove("error");
    bubble.classList.remove("warning");
    bubble.classList.remove("info");
    bubble.classList.add(level);
    bubble.innerHTML = html;

    // TODO(emilio): Old code animated the bubble.
    bubble.style.display = "block";
  }

  hideBubble(which) {
    // TODO(emilio): Old code animated the bubble.
    this.bubbles[which].style.display = "none";
  }

  hideBubbles() {
    for (let kind in this.bubbles) {
      this.hideBubble(kind);
    }
  }

  setupClassSorter() {
    if (!this.colSelector) {
      return;
    }

    const button = document.querySelector("#reorder-classes");
    if (!button) {
      return;
    }

    // NOTE: There can be multiple tables.

    const rowsListList = [];
    for (const tbody of document.querySelectorAll("#symbol-tree-table-list tbody")) {
      const rowsList = [];
      let currentRows = null;
      for (const row of tbody.querySelectorAll("tr")) {
        const firstCell = row.querySelector("td");
        if (!firstCell) {
          continue;
        }
        if (firstCell.classList.contains("base-class-false") ||
            firstCell.classList.contains("base-class-true")) {
          currentRows = [row];
          rowsList.push(currentRows);
          continue;
        }
        if (!currentRows) {
          continue;
        }
        currentRows.push(row);
      }
      rowsListList.push({ tbody, rowsList });
    }

    let useAscending = true;

    button.addEventListener("click", () => {
      useAscending = !useAscending;

      if (useAscending) {
        button.textContent = "Use Ascending Order";
      } else {
        button.textContent = "Use Descending Order";
      }

      for (const { tbody, rowsList } of rowsListList) {
        let list;
        if (useAscending) {
          list = rowsList;
        } else {
          list = rowsList.toReversed();
        }

        for (const rows of list) {
          for (const row of rows) {
            tbody.append(row);
          }
        }
      }
    });
  }

  setupColSelector() {
    this.colSelector = document.querySelector("#symbol-tree-table-col-selector");
    if (!this.colSelector) {
      return;
    }
    this.symbolTreeTableList = document.querySelector("#symbol-tree-table-list");
    if (!this.symbolTreeTableList) {
      return;
    }

    const defaultCols = {
      name: true,
      type: false,
      line: true,
    };

    this.cols = {};
    for (const [key, defaultValue] of Object.entries(defaultCols)) {
      const node = document.querySelector("#col-show-" + key);
      node.addEventListener("change", () => {
        this.onColChange();
      });
      this.cols[key] = {
        node: node,
        currentValue: defaultValue,
        defaultValue: defaultValue,
      };
    }

    this.parseColQuery();
  }

  parseColQuery() {
    if (!this.colSelector) {
      return;
    }

    let query = this.fields.query.value;

    for (const m of query.matchAll(/(show|hide)-cols:([a-z,]+)/g)) {
      const show = m[1] == "show";
      const cols = m[2].split(/,/);

      for (const col of cols) {
        this.cols[col].currentValue = show;
      }
    }

    this.updateColCheckbox();
  }

  updateColCheckbox() {
    for (const [key, obj] of Object.entries(this.cols)) {
      obj.node.checked = obj.currentValue;
    }
  }

  onColChange() {
    for (const [key, obj] of Object.entries(this.cols)) {
      obj.currentValue = obj.node.checked;

      if (!obj.defaultValue) {
        this.symbolTreeTableList.classList.toggle("show-" + key, obj.currentValue);
      } else {
        this.symbolTreeTableList.classList.toggle("hide-" + key, !obj.currentValue);
      }
    }

    this.updateColQuery();
  }

  getShowCols() {
    const showCols = [];
    for (const [key, obj] of Object.entries(this.cols)) {
      if (obj.currentValue != obj.defaultValue && !obj.defaultValue) {
        showCols.push(key);
      }
    }
    return showCols.join(",");
  }

  getHideCols() {
    const hideCols = [];
    for (const [key, obj] of Object.entries(this.cols)) {
      if (obj.currentValue != obj.defaultValue && obj.defaultValue) {
        hideCols.push(key);
      }
    }
    return hideCols.join(",");
  }

  updateColQuery() {
    const showCols = this.getShowCols();
    const hideCols = this.getHideCols();

    let query = this.fields.query.value;
    query = query.replace(/ +(show|hide)-cols:([a-z,]+)/g, "");

    if (showCols) {
      query += " show-cols:" + showCols;
    }
    if (hideCols) {
      query += " hide-cols:" + hideCols;
    }

    this.fields.query.value = query;
    let url = this.constructURL();
    this.updateHistory(url);
  }
})();

var Diagram = new (class Diagram {
  LABELS = {
    "Pointer strength": {
      "\u{1f4aa}": {
        // from kind = "strong"
        desc: "Strong pointer",
      },
      "\u{2744}\u{fe0f}": {
        // from kind = "unique"
        desc: "Unique pointer",
      },
      "\u{1f4d3}\u{fe0f}": {
        // from kind = "weak"
        desc: "Weak pointer",
      },
      "\u{1f631}": {
        // from kind = "raw"
        desc: "Raw pointer",
      },
      "&": {
        // from kind = "ref"
        desc: "Reference",
      },
      "\u{1fada}": {
        // from kind = "gcref"
        desc: "GC reference",
      },
      "\u{1f4e6}": {
        // from kind = "contains"
        desc: "Contains",
      },
    },
    "Classes and fields": {
      "\u{269b}\u{fe0f}": {
        // from label = "arc" or label = "atomic"
        desc: "Atomic or Atomic reference counted class",
      },
      "\u{1f517}": {
        // from label = "cc"
        desc: "Cycle-collected class",
      },
      "\u{26d3}\u{fe0f}": {
        // from label = "ccrc"
        desc: "Cycle-collected reference counted class",
      },
      "\u{1f517}\u{270f}\u{fe0f}": {
        // from label = "cc-trace"
        desc: "Field referenced in ::cycleCollection::Trace",
      },
      "\u{1f517}\u{1f50d}": {
        // from label = "cc-traverse"
        desc: "Field referenced in ::cycleCollection::Traverse",
      },
      "\u{26d3}\u{fe0f}\u{200d}\u{1f4a5}": {
        // from label = "cc-unlink"
        desc: "Field referenced in ::cycleCollection::Unlink",
      },
      "\u{1f9ee}": {
        // from label = "rc"
        desc: "Reference counted class",
      },
    },
    "Interfaces and super classes": {
      "nsIIReq": {
        // from elide-and-badge
        desc: "nsIInterfaceRequestor",
      },
      "nsIObs": {
        // from elide-and-badge
        desc: "nsIObserver",
      },
      "nsIRun": {
        // from elide-and-badge
        desc: "nsIRunnable",
      },
      "nsI": {
        // from elide-and-badge
        desc: "nsISupports",
      },
      "nsSupWeak": {
        // from elide-and-badge
        desc: "nsSupportsWeakReference",
      },
      "WC": {
        // from elide-and-badge
        desc: "nsWrapperCache",
      },
    },
  };

  constructor() {
    this.addControl();
    this.addBadgeTooltips();

    this.setScrollPosition();
  }

  addControl() {
    this.panel = null;
    this.ignoreNodesItem = null;

    if (typeof GRAPH_OPTIONS == "undefined") {
      return;
    }

    this.panel = document.querySelector("#diagram-panel");

    const optionsPane = document.createElement("div");
    optionsPane.id = "diagram-options-pane";

    for (const { section, items } of GRAPH_OPTIONS) {
      const sectionLabel = document.createElement("h3");
      sectionLabel.append(section);
      optionsPane.append(sectionLabel);
      const sectionBox = document.createElement("div");
      sectionBox.classList.add("diagram-panel-section");

      for (const item of items) {
        const label = document.createElement("label");
        label.id = "diagram-option-label-" + item.name;
        label.setAttribute("for", "diagram-option-" + item.name);
        label.append(item.label);
        sectionBox.append(label);

        if ("choices" in item) {
          const select = document.createElement("select");
          select.id = "diagram-option-" + item.name;
          select.setAttribute("aria-labelledby", "diagram-option-label-" + item.name);
          for (const choice of item.choices) {
            const option = document.createElement("option");
            option.value = choice.value;
            option.append(choice.label);
            select.append(option);
          }
          select.value = item.value;

          select.addEventListener("change", () => {
            item.value = select.value;
          });

          sectionBox.append(select);
        } else if ("range" in item) {
          const box = document.createElement("span");

          const min = document.createElement("span");
          min.classList.add("diagram-panel-range-min");
          min.append(item.range[0]);
          box.append(min);

          const range = document.createElement("input");
          range.id = "diagram-option-range-" + item.name;
          range.classList.add("diagram-panel-range");
          range.type = "range";
          range.min = item.range[0];
          range.max = item.range[1];
          range.value = item.value;
          box.append(range);

          const max = document.createElement("span");
          max.classList.add("diagram-panel-range-max");
          max.append(item.range[1]);
          box.append(max);

          const input = document.createElement("input");
          input.id = "diagram-option-" + item.name;
          input.setAttribute("aria-labelledby", "diagram-option-label-" + item.name);
          input.size = 4;
          input.type = "text";
          input.value = item.value;
          box.append(input);

          input.addEventListener("input", () => {
            item.value = input.value;
            range.value = item.value;
          });
          range.addEventListener("input", () => {
            item.value = range.value;
            input.value = item.value;
          });

          sectionBox.append(box);
        } else if ("type" in item && item.type == "string") {
          const input = document.createElement("input");
          input.id = "diagram-option-" + item.name;
          input.value = item.value;
          input.placeholder = item.placeholder;

          input.addEventListener("input", () => {
            item.value = input.value;
          });

          sectionBox.append(input);

          if (item.name == "ignore-nodes") {
            this.ignoreNodesItem = item;
          }
        } else if ("type" in item && item.type == "bool") {
          const input = document.createElement("input");
          input.id = "diagram-option-" + item.name;
          input.setAttribute("aria-labelledby", "diagram-option-label-" + item.name);
          input.type = "checkbox";
          if (item.value) {
            input.checked = true;
          }

          input.addEventListener("change", () => {
            item.value = input.checked;
          });

          sectionBox.append(input);
        } else {
          const unknown = document.createElement("div");
          unknown.append("(unknown)");
          sectionBox.append(unknown);
        }
      }
      optionsPane.append(sectionBox);
    }

    const apply = document.createElement("button");
    apply.append("Apply");

    apply.addEventListener("click", () => {
      this.applyOptions();
    });

    optionsPane.append(apply);
    this.panel.append(optionsPane);

    const legendPane = document.createElement("div");
    legendPane.id = "diagram-legend-pane";

    const legendTitle = document.createElement("h3");
    legendTitle.append("Legend");
    legendPane.append(legendTitle);

    for (const [sectionLabel, section] of Object.entries(this.LABELS)) {
      const legendTitle = document.createElement("h4");
      legendTitle.append(sectionLabel);
      legendPane.append(legendTitle);

      const legend = document.createElement("div");
      legend.classList.add("diagram-legend");
      for (const [label, item] of Object.entries(section)) {
        const labelBox = document.createElement("span");
        labelBox.append(label);
        if (label.codePointAt(0) > 0x7f) {
          labelBox.style.fontSize = "1.2em";
        }
        legend.append(labelBox);

        const descBox = document.createElement("span");
        descBox.append(item.desc);
        legend.append(descBox);
      }
      legendPane.append(legend);
    }

    this.panel.append(legendPane);
  }

  liftLimit(kind, exists) {
    switch (kind) {
      case "UsesPaths":
      case "UsesLines": {
        const item = this.getOption("path-limit");
        if (item) {
          this.setOption(item, exists + 100);
        }
        break;
      }
      case "NodeLimit": {
        const item = this.getOption("node-limit") ||
              this.getOption("paths-between-node-limit");
        if (item) {
          this.setOption(item, item.range[1]);
        }
        break;
      }
      case "Overrides":
      case "Subclasses":
      case "FieldMemberUses":
        // Unsupported.
        return;
    }

    this.applyOptions();
  }

  getOption(name) {
    for (const { section, items } of GRAPH_OPTIONS) {
      for (const item of items) {
        if (item.name === name) {
          return item;
        }
      }
    }
    return null;
  }

  setOption(item, value) {
    item.value = value;
    if ("range" in item) {
      item.value = Math.max(item.value, item.range[0]);
      item.value = Math.min(item.value, item.range[1]);
    }
    return true;
  }

  applyOptions() {
    let query = Dxr.fields.query.value;

    for (const { section, items } of GRAPH_OPTIONS) {
      for (const item of items) {
        const re = new RegExp(" +" + item.name + ":[^ ]+");
        query = query.replace(re, "");
        if (item.value != item.default) {
          query += " " + item.name + ":" + item.value;
        }
      }
    }

    Dxr.fields.query.value = query;
    let url = Dxr.constructURL();
    document.location = url;
  }

  togglePanel() {
    if (!this.panel) {
      return;
    }
    this.panel.classList.toggle("hidden");
  }

  addBadgeTooltips() {
    for (const text of document.querySelectorAll(`svg text[text-decoration="underline"]`)) {
      const label = text.textContent;
      for (const section of Object.values(this.LABELS)) {
        if (label in section) {
          const desc = section[label].desc;

          let tooltip = null;

          text.addEventListener("mouseenter", () => {
            if (tooltip) {
              tooltip.remove();
              tooltip = null;
            }

            const rect = text.getBoundingClientRect();
            const x = rect.left + window.scrollX;
            const y = rect.bottom + window.scrollY;

            tooltip = document.createElement("div");
            tooltip.classList.add("diagram-badge-tooltip");
            tooltip.style.left = (x - 16) + "px";
            tooltip.style.top = (y + 8) + "px";

            const main = document.createElement("div");
            main.classList.add("diagram-badge-tooltip-main");
            main.append(desc);
            tooltip.append(main);

            const arrowBox = document.createElement("div");
            arrowBox.classList.add("diagram-badge-tooltip-arrow-box");

            const arrow = document.createElement("div");
            arrow.classList.add("diagram-badge-tooltip-arrow");
            arrowBox.append(arrow);
            tooltip.append(arrowBox);

            document.body.append(tooltip);
          });
          text.addEventListener("mouseleave", () => {
            if (tooltip) {
              tooltip.remove();
              tooltip = null;
            }
          });
        }
      }
    }
  }

  canIgnoreNode() {
    return this.panel && this.ignoreNodesItem;
  }

  ignoreNode(pretty) {
    if (this.ignoreNodesItem.value != "") {
      this.ignoreNodesItem.value += "," + pretty;
    } else {
      this.ignoreNodesItem.value = pretty;
    }
    this.applyOptions();
  }

  setScrollPosition() {
    // Scroll the first root node of the diagram so that it's centered.
    // Through use of `?.` this won't freak out if there are no matches.
    // According to performance.now(), this takes 3ms on the 20,765 line
    // indexedDB/ActorsParent.cpp right now.
    //
    // This file is loaded at the bottom of the HTML file so the DOM is
    // available, although I'm not entirely sure this is wise versus hooking
    // the load event.
    //
    // TODO: Be more wise.
    document.querySelector(".diagram-depth-0 polygon")?.scrollIntoView({
      behavior: "instant",
      block: "center",
      inline: "center"
    });
  }
})();

function hashString(string) {
  let hash = 0;
  if (string.length == 0) {
    return hash;
  }
  for (let i = 0; i < string.length; i++) {
    let char = string.charCodeAt(i);
    hash = (hash << 5) - hash + char;
    hash = hash & hash; // Convert to 32bit integer
  }
  return hash;
}

function classOfResult(pathkind, qkind, isContext) {
  var klass = pathkind + ":" + qkind;
  let cssClass = "EXPANDO" + hashString(klass);
  if (isContext) {
    cssClass += ` ${isContext}-context-line`;
  }
  return cssClass;
}

function onExpandoClick(event) {
  let target = event.target;
  let wasOpen = target.classList.contains("open");
  let elements = document.querySelectorAll(
    "." + target.getAttribute("data-klass")
  );
  for (const element of elements) {
    element.style.display = wasOpen ? "none" : "";
  }
  target.classList.toggle("open");
  target.innerHTML = wasOpen ? "&#9654;" : "&#9660;";

  if (wasOpen) {
    return;
  }

  for (const element of elements) {
    const nestedExpando = element.querySelector(".expando");

    if (!nestedExpando || nestedExpando.classList.contains("open")) {
      continue;
    }

    const nestedClass = nestedExpando.getAttribute("data-klass");
    const nestedRows = document.querySelectorAll("." + nestedClass);
    for (const row of nestedRows) {
      row.style.display = "none";
    }
  }
}

var populateEpoch = 0;
function populateResults(data, full, jumpToSingle) {
  populateEpoch++;

  // We use to delete these fields from the data, but that's not compatible of
  // the current 2-pass rendering approach, so now the logic below knows to skip
  // any key that starts with a "*".
  var title = data["*title*"];
  if (title) {
    document.title = title + " - mozsearch";

    // Tell the title to webtest.
    document.dispatchEvent(new Event("titlechanged"));
  }
  var timed_out = data["*timedout*"];

  let limits_hit = data["*limits*"] || [];

  let nsresult = data["*nsresult*"];

  window.scrollTo(0, 0);

  function makeURL(path) {
    return "/" + Dxr.tree + "/source/" + encodeURI(path);
  }

  function makeSearchUrl(q) {
    return `/${Dxr.tree}/search?q=${encodeURIComponent(q)}`;
  }

  function chooseIcon(path) {
    var suffix = path.lastIndexOf(".");
    if (suffix == -1) {
      return "unknown";
    }
    suffix = path.slice(suffix + 1);
    return (
      {
        cpp: "cpp",
        h: "h",
        c: "c",
        m: "mm",
        mm: "mm",
        js: "js",
        jsm: "js",
        mjs: "js",
        py: "py",
        ini: "conf",
        sh: "sh",
        txt: "txt",
        xml: "xml",
        xul: "ui",
        java: "java",
        in: "txt",
        html: "html",
        png: "image",
        gif: "image",
        svg: "svg",
        build: "build",
        json: "js",
        css: "css",
      }[suffix] || "unknown"
    );
  }

  function renderPath(pathkind, qkind, fileResult, fileSpecificClass) {
    var klass = classOfResult(pathkind, qkind);

    var html = "";
    html += "<tr class='result-head " + klass + "'>";
    html += "<td class='left-column'>";

    if (fileResult.lines && fileResult.lines.length > 0) {
      html += "<div class='expando open' data-klass='" + fileSpecificClass + "'>&#9660;</div>";
    } else {
      html += "<div class='expando' style='visibility: hidden'>&#9660;</div>";
    }

    html += "<div class='mimetype-icon-" +
      chooseIcon(fileResult.path) +
      " mimetype-floating-container'></div></td>";

    // This span exists for a11y reasons.  See bug 1558691 but the core idea is:
    // - It's fine (and good!) to upgrade this to an explicit h3 in the future.
    //   We should always favor semantic HTML over use of divs/spans.
    // - At the current moment we're just introducing the span because the
    //   styling fallout is potentially more than we want to get into.
    html += `<td><span role="heading" aria-level="3">`;

    var elts = fileResult.path.split("/");
    var pathSoFar = "";
    for (var i = 0; i < elts.length; i++) {
      if (i != 0) {
        html += "<span class='path-separator'>/</span>";
      }

      var elt = elts[i];
      pathSoFar += elt;
      html += "<a href='" + makeURL(pathSoFar) + "'>" + elt + "</a>";
      pathSoFar += "/";
    }

    html += "</span></td>";
    html += "</tr>";

    return html;
  }

  function escape(s) {
    return s.replace(/&/gm, "&amp;").replace(/</gm, "&lt;");
  }

  function renderSingleSearchResult(pathkind, qkind, file, line, isContext, hasContext, fileSpecificClass) {
    var [start, end] = line.bounds || [0, 0];
    var before = line.line.slice(0, start);
    // Do not truncate off the leading whitespace if we're trying to present in context.
    if (!hasContext) {
      before = before.replace(/^\s+/, "");
    }
    var middle = line.line.slice(start, end);
    var after = line.line.slice(end).replace(/\s+$/, "");

    var klass = classOfResult(pathkind, qkind, isContext) + " " + fileSpecificClass;
    var html = "";
    html += "<tr class='" + klass + "'>";
    html +=
      "<td class='left-column'><a href='" +
      makeURL(file.path) +
      "#" +
      line.lno +
      "'>" +
      line.lno +
      "</a></td>";
    html += "<td><a href='" + makeURL(file.path) + "#" + line.lno + "'>";

    html += "<code>";
    // in the context cases, we may only have an after.
    if (before) {
      html += escape(before);
    }
    if (middle) {
      html += "<b>" + escape(middle) + "</b>";
    }
    html += escape(after);
    html += "</code>";

    html += "</a>";

    if (line.context) {
      var inside = line.context;
      if (line.contextsym) {
        var url = `/${Dxr.tree}/search?q=symbol:${encodeURIComponent(
          line.contextsym
        )}&redirect=false`;
        inside = "<a href='" + url + "'>" + escape(line.context) + "</a>";
      }
      html +=
        " <span class='result-context'>// found in <code>" +
        inside +
        "</code></span>";
    }

    // Hacky attempt to provide a means of providing related searches.
    if (line.upsearch) {
      html += `<span class='result-upsearch'><a href="${makeSearchUrl(
        line.upsearch
      )}">Symbol Search This</a></span>`;
    }

    html += "</td>";
    html += "</tr>";

    return html;
  }

  // Accumulate the total number of results for our initial summary and to
  // support our "automatically go to a single result" UX optimization below.
  var count = 0;
  var tupledCounts = new Map();
  var pathkindCounts = new Map();
  for (var pathkind in data) {
    // Skip metadata fields.  The current idiom calls this method twice to
    // reduce initial above-the-folder rendering into place, which means that
    // we can't just delete these fields.
    if (pathkind.startsWith("*")) {
      continue;
    }
    let pathkindHits = 0;
    let pathkindFiles = 0;
    let pathkindFilenameMatches = 0;
    for (var qkind in data[pathkind]) {
      let qkindHitcount = 0;
      for (var k = 0; k < data[pathkind][qkind].length; k++) {
        var path = data[pathkind][qkind][k];
        // 0 lines implies this is a file, in which case just the bare file still
        // counts for our result count purposes and how router.py determined the
        // limits.
        if (!path.lines.length) {
          pathkindFilenameMatches++;
        }
        count += (path.lines.length || 1);
        qkindHitcount += path.lines.length;
      }
      tupledCounts.set(`${pathkind}-${qkind}`, { hits: qkindHitcount, files: data[pathkind][qkind].length });
      pathkindHits += qkindHitcount;
      pathkindFiles += data[pathkind][qkind].length;
    }

    pathkindCounts.set(pathkind, { hits: pathkindHits, files: pathkindFiles, filenameMatches: pathkindFilenameMatches });
  }

  // Accumulate the total number of files to support our "automatically go to a
  // single result" UX optimization below.
  var fileCount = 0;
  for (var pathkind in data) {
    if (pathkind.startsWith("*")) {
      continue;
    }
    for (var qkind in data[pathkind]) {
      fileCount += data[pathkind][qkind].length;
    }
  }

  // If there's only a single result, redirect ourselves there directly.
  if (jumpToSingle && fileCount == 1 && count <= 1) {
    var pathkind = Object.keys(data)[0];
    var qkind = Object.keys(data[pathkind])[0];
    var file = data[pathkind][qkind][0];
    var path = file.path;

    if (count == 1) {
      var line = file.lines[0];
      // If there was only a "file" response just act like we clicked on the
      // file def by using a line number of 1.
      var lno = line?.lno || 1;
      window.location = `/${Dxr.tree}/source/${path}#${lno}`;
    } else {
      window.location = `/${Dxr.tree}/source/${path}`;
    }
    return;
  }

  // If no data is returned, inform the user.
  let container = document.getElementById("content");
  // Clobber any additional classes that existed on the content container.
  container.setAttribute("class", "content");

  const items = [];

  const breadcrumbs = document.querySelector(".breadcrumbs");
  if (breadcrumbs) {
    // Preserve breadcrumbs if present.
    items.push(breadcrumbs);

    // Breadcrumbs is hidden in some page.
    breadcrumbs.style.display = "inline-block";

    // Remove path and symbols.
    //
    // NOTE: Search can be initiated from source or directory listing,
    //       where breadcrumbs has path for the file or directory.
    let foundSep = false;
    for (const node of [...breadcrumbs.childNodes]) {
      if (node instanceof HTMLElement) {
        if (node.classList.contains("path-separator")) {
          foundSep = true;
        }
      }
      if (foundSep) {
        breadcrumbs.removeChild(node);
      }
    }
  }
  const navigationPanel = document.querySelector("#panel");
  if (navigationPanel) {
    // Preserve navigation panel if present.
    items.push(navigationPanel);

    try {
      Panel.prepareForSearch();
    } catch {}
  }

  const header = document.createElement("div");
  header.classList.add("search-result-header");
  items.push(header);

  if (!fileCount) {
    const div = document.createElement("div");
    div.textContent = "No results for current query.";
    header.append(div);
  } else {
    if (count) {
      const div = document.createElement("div");
      div.textContent = `Number of results: ${count} (maximum is around 4000)`;
      header.append(div);
    }
  }

  if (limits_hit.length > 0) {
    const div = document.createElement("div");
    const b = document.createElement("b");
    b.textContent = "Warning";
    div.append(b);
    div.append(`: The following limits were hit in your search: ${limits_hit.join(", ")}`);
    header.append(div);
  }

  let timeoutWarning = null;
  if (timed_out) {
    const div = document.createElement("div");
    const b = document.createElement("b");
    b.textContent = "Warning";
    div.append(b);
    div.append(": Results may be incomplete due to server-side search timeout!");
    header.append(div);
  }

  if (nsresult) {
    function toHex(n) {
      return "0x" + n.toString(16);
    }
    function createCode(code) {
      const c = document.createElement("code");
      c.textContent = `${escape(code)}`;
      return c;
    }
    function createCodeLink(code) {
      const link = document.createElement("a");
      link.href = makeSearchUrl(code);
      const c = document.createElement("code");
      c.textContent = `${escape(code)}`;
      link.append(c);
      return link;
    }

    const div = document.createElement("div");
    div.classList.add("nsresult-desc");
    const c = document.createElement("code");
    if (typeof nsresult.raw_code === "number") {
      div.append(createCode(`nsresult(${toHex(nsresult.raw_code)})`));
    } else {
      div.append(createCode(`nsresult(${nsresult.query || ""})`));
    }
    div.append(" is ");
    if (Array.isArray(nsresult.codes)) {
      let first = true;
      for (const code of nsresult.codes) {
        if (!first) {
          div.append(" / ");
        }
        first = false;
        div.append(createCodeLink(code));
      }
    } else if (typeof nsresult.sev == "string" && typeof nsresult.raw_subcode == "number") {
      if (typeof nsresult.mod == "string") {
        div.append(createCodeLink("NS_ERROR_GENERATE"));
        div.append(createCode("("));
        div.append(createCodeLink(nsresult.sev));
        div.append(createCode(", "));
        div.append(createCodeLink(nsresult.mod));
        div.append(createCode(`, ${toHex(nsresult.raw_subcode)})`));
      } else if (typeof nsresult.raw_mod == "number") {
        div.append(createCodeLink("NS_ERROR_GENERATE"));
        div.append(createCode("("));
        div.append(createCodeLink(nsresult.sev));
        div.append(createCode(`, ${toHex(nsresult.raw_mod)}, ${toHex(nsresult.raw_subcode)})`));
      } else {
        div.append("unknown");
      }
    } else {
      div.append("unknown");
    }
    header.append(div);
  }

  if (fileCount) {
    var table = document.createElement("table");
    table.className = "results";
    items.push(table);

    var counter = 0;

    var pathkindNames = {
      // Previously we would not say normal, but we need a place to hang the
      // counts.
      normal: "Core code",
      test: "Test files",
      generated: "Generated code",
      thirdparty: "Third-party code",
    };

    var html = "";
    // Loop over normal/test/generated/thirdparty "pathkind"s
    for (var pathkind in data) {
      if (pathkind.startsWith("*")) {
        continue;
      }
      var pathkindName = pathkindNames[pathkind];
      if (pathkindName) {
        let pathkindCount = pathkindCounts.get(pathkind);
        if (pathkindCount) {
          if (pathkindCount.hits) {
            if (pathkindCount.filenameMatches > 0) {
              const plural = pathkindCount.filenameMatches === 1 ? "" : "s";
              maybeCounts = ` (${pathkindCount.filenameMatches} filename${plural} and ${pathkindCount.hits} lines across ${pathkindCount.files} files)`;
            } else {
              maybeCounts = ` (${pathkindCount.hits} lines across ${pathkindCount.files} files)`;
            }
          } else {
            maybeCounts = ` (${pathkindCount.files} files)`;
          }
        }

        html += "<tr><td>&nbsp;</td></tr>";
        html +=
          "<tr><td class='section'>§</td><td><div class='result-pathkind'>" +
          pathkindName + maybeCounts +
          "</div></td></tr>";
      }

      const pathKeys = Object.keys(data[pathkind]);
      const hasSymbolicIdentity = pathKeys.some(k =>
        k.startsWith("Definitions") ||
        k.startsWith("Declarations") ||
        k.startsWith("IDL")
      );

      const hasRealUses = pathKeys.some(k => k.startsWith("Uses") && data[pathkind][k].length > 0);

      let noUsesRendered = false;

      const getNoUsesHtml = () => {
        const uniqueToggleClass = "no-uses-toggle-" + pathkind;

        let str = "";
        str += "<tr><td>&nbsp;</td></tr>";

        str += "<tr>";
        str +=   "<td class='left-column'>";
        str +=     "<div class='expando open' data-klass='" + uniqueToggleClass + "'>&#9660;</div>";
        str +=   "</td>";
        str +=   "<td>";
        str +=     "<h2 class='result-kind'>Uses (0)</h2>";
        str +=   "</td>";
        str += "</tr>";
        str += "<tr class='" + uniqueToggleClass + "'>";
        str +=   "<td class='left-column'></td>";
        str +=   "<td>";
        str +=     "<div class='result-no-uses-note'>";
        str +=       "No uses found.";
        str +=       "<small>";
        str +=         "Note: Some template-heavy or indirect usages may not be fully indexed.";
        str +=       "</small>";
        str +=     "</div>";
        str +=   "</td>";
        str += "</tr>";
        return str;
      };

      // Loop over definition/declaration/use/etc. "qkind"s
      for (var qkind in data[pathkind]) {
        if (qkind === "Textual Occurrences" && hasSymbolicIdentity && !hasRealUses && !noUsesRendered) {
          html += getNoUsesHtml();
          noUsesRendered = true;
        }

        if (data[pathkind][qkind].length) {
          html += "<tr><td>&nbsp;</td></tr>";

          html += "<tr><td class='left-column'>";
          html +=
            "<div class='expando open' data-klass='" +
            classOfResult(pathkind, qkind) +
            "'>&#9660;</div>";
          html += "</td>";

          let maybeCounts = "";
          let qkindCount = tupledCounts.get(`${pathkind}-${qkind}`);
          if (qkindCount) {
            if (qkindCount.hits) {
              maybeCounts = ` (${qkindCount.hits} lines across ${qkindCount.files} files)`;
            } else {
              maybeCounts = ` (${qkindCount.files} files)`;
            }
          }
          html += "<td><h2 class='result-kind'>" + escape(qkind) + maybeCounts + "</h2></td></tr>";
        }

        // Loop over the files with hits.
        for (var i = 0; i < data[pathkind][qkind].length; i++) {
          var file = data[pathkind][qkind][i];

          if (counter > 100 && !full) {
            break;
          }

          var fileSpecificClass = "FILE_" + hashString(file.path + pathkind + qkind);
          html += renderPath(pathkind, qkind, file, fileSpecificClass);

          file.lines.map(function (line) {
            counter++;
            if (counter > 100 && !full) {
              return;
            }

            let has_context = line.context_before || line.context_after;

            if (line.context_before) {
              let lineDelta = -line.context_before.length;
              for (const lineStr of line.context_before) {
                html += renderSingleSearchResult(
                  pathkind,
                  qkind,
                  file,
                  { lno: line.lno + lineDelta, line: lineStr },
                  "before",
                  true,
                  fileSpecificClass
                );
                lineDelta++;
              }
            }
            html += renderSingleSearchResult(pathkind, qkind, file, line, false, has_context, fileSpecificClass);
            if (line.context_after) {
              let lineDelta = 1;
              for (const lineStr of line.context_after) {
                html += renderSingleSearchResult(
                  pathkind,
                  qkind,
                  file,
                  { lno: line.lno + lineDelta, line: lineStr },
                  "after",
                  true,
                  fileSpecificClass
                );
                lineDelta++;
              }
            }
          });
        }
      }

      if (hasSymbolicIdentity && !hasRealUses && !noUsesRendered) {
        html += getNoUsesHtml();
      }
    }

    table.innerHTML = html;

    for (let element of table.querySelectorAll(".expando")) {
      element.addEventListener("click", onExpandoClick);
    }

    if (counter > 100 && !full) {
      var epoch = populateEpoch;
      setTimeout(function () {
        if (populateEpoch == epoch) {
          populateResults(data, true, false);
        }
      }, 750);
    }
  }

  container.replaceChildren(...items);

  Dxr.parseColQuery();
}

window.showSearchResults = function (results) {
  var jumpToSingle = window.location.search.indexOf("&redirect=false") == -1;
  populateResults(results, true, jumpToSingle);
};

/**
 * Adds a leading 0 to numbers less than 10 and greater that 0
 *
 * @param int number The number to test against
 *
 * return Either the original number or the number prefixed with 0
 */
function addLeadingZero(number) {
  return number <= 9 || number === 0 ? "0" + number : number;
}

/**
 * Converts string to new Date and returns a formatted date in the
 * format YYYY-MM-DD h:m
 * @param String dateString A date in string form.
 *
 */
function formatDate(dateString) {
  var fullDateTime = new Date(dateString),
    date =
      fullDateTime.getFullYear() +
      "-" +
      (fullDateTime.getMonth() + 1) +
      "-" +
      addLeadingZero(fullDateTime.getDate()),
    time =
      fullDateTime.getHours() + ":" + addLeadingZero(fullDateTime.getMinutes());

  return date + " " + time;
}

for (let element of document.querySelectorAll(".pretty-date")) {
  element.innerText = formatDate(element.getAttribute("data-datetime"));
}
