"use strict";

// A try push's canvas (`/{tree}/rev-canvas/{head}`; see
// `format::format_try_canvas` and `patch_canvas`): the functions it adds or
// changes, as cards of their code, connected by the calls among them and with
// the functions calling them or called by them (the context), laid out by their
// calls with graph.js.  Hovering over a call shows the lines making it.
//
// The model (CANVAS) is the server's and the same for everyone; its
// arrangement is the user's (`state`): whether the context is shown, which
// cards are hidden, and which are collapsed to their names.  It's kept in this
// browser (IndexedDB, by tree and head, with when it was saved), and can be
// exported as JSON (which names the push, and its model's URL) and imported,
// ex: to share it.

var RevCanvas = new (class RevCanvas {
  constructor() {
    window.addEventListener("load", () => this.init(), { once: true });
  }

  static FORMAT = "searchfox-rev-canvas";
  static VERSION = 1;
  static DB = "searchfox-rev-canvas";
  static STORE = "arrangements";

  defaultState() {
    return { context: true, hidden: [], collapsed: [] };
  }

  async init() {
    if (typeof CANVAS === "undefined") {
      return;
    }
    // Make the page fit the content area, so that no scrollbar is shown.
    document.documentElement.classList.add("for-interactive-graph");
    this.model = CANVAS;
    this.key = `${this.model.tree}/${this.model.head}`;
    this.nodes = new Map(this.model.nodes.map(node => [node.id, node]));
    // (By their edges' `data-symbols`.)
    this.calls = new Map(this.model.edges.map(edge => [`${edge.from}->${edge.to}`, edge]));
    this.state = this.defaultState();
    this.buildToolbar();
    this.graph = new Graph(
      document.querySelector("#interactive-graph-viewport"),
      document.querySelector("#interactive-graph-container"),
      this.input(), [], [], GRAPH_EXTRA,
      { renderBlock: (block, node) => this.renderBlock(block, node) });
    this.addCallHovers();
    this.updateToolbar();
    // (Then as this browser last arranged it, if it did.)
    try {
      const saved = await this.load(this.key);
      if (saved) {
        this.state = this.cleanState(saved.state);
        this.savedAt = saved.saved;
        this.graph.setInput(this.input());
        this.updateToolbar();
      }
    } catch (e) {
      console.warn("Couldn't load the canvas's arrangement:", e);
    }
    document.documentElement.dataset.canvasReady = "true";
  }

  // A state's known fields, of the model's nodes.
  cleanState(state) {
    const ids = list => (Array.isArray(list) ? list : []).filter(id => this.nodes.has(id));
    return {
      context: state?.context !== false,
      hidden: ids(state?.hidden),
      collapsed: ids(state?.collapsed),
    };
  }

  // The graph's input: the shown cards, and the calls among them.
  input() {
    const hidden = new Set(this.state.hidden);
    const shown = new Set(
      this.model.nodes
        .filter(node => (this.state.context || node.status != "context") && !hidden.has(node.id))
        .map(node => node.id));
    return {
      nodes: this.model.nodes
        .filter(node => shown.has(node.id))
        .map(node => ({ sym: node.id, pretty: node.pretty, node })),
      edges: this.model.edges
        .filter(edge => shown.has(edge.from) && shown.has(edge.to))
        .map(edge => ({
          from: edge.from,
          to: edge.to,
          kind: "default",
          // (The calls' lines, for the edge's context menu.)
          jumps: [`${this.nodes.get(edge.from).path}#${edge.lines.join(",")}`],
          hovers: [],
        })),
    };
  }

  renderBlock(block, element) {
    const node = block.data.node;
    const collapsed = this.state.collapsed.includes(node.id);
    element.classList.add("rc-card", `rc-status-${node.status}`);
    element.dataset.canvasNode = node.id;

    const head = document.createElement("div");
    head.className = "rc-card-head";
    const status = document.createElement("span");
    status.className = "rc-badge";
    status.textContent = node.status;
    const name = document.createElement("span");
    name.className = "rc-name";
    name.textContent = node.pretty;
    const where = document.createElement("a");
    where.className = "rc-where";
    where.href = `${node.link}#${node.lines[0]}`;
    where.textContent = `${node.path.split("/").pop()}:${node.lines[0]}`;
    where.title = node.path;
    // (Not the card's context menu.)
    where.addEventListener("click", event => event.stopPropagation());
    const collapse = this.button(collapsed ? "▸" : "▾",
                                 collapsed ? "Show its code" : "Show only its name",
                                 () => this.toggle("collapsed", node.id));
    collapse.classList.add("rc-collapse");
    const hide = this.button("×", "Hide it", () => this.toggle("hidden", node.id));
    hide.classList.add("rc-hide");
    head.append(collapse, status, name, where, hide);
    element.append(head);

    if (!collapsed && node.rows) {
      const rows = document.createElement("div");
      rows.className = "file rc-rows";
      rows.setAttribute("role", "table");
      rows.innerHTML = node.rows;
      element.append(rows);
      if (!node.complete) {
        const more = document.createElement("a");
        more.className = "rc-more";
        more.href = `${node.link}#${node.lines[0]}-${node.lines[1]}`;
        more.textContent = `… lines ${node.lines[0]}-${node.lines[1]} in the file`;
        element.append(more);
      }
    }
  }

  button(text, title, onclick) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = text;
    button.title = title;
    button.addEventListener("click", event => {
      event.preventDefault();
      event.stopPropagation();
      onclick();
    });
    return button;
  }

  // Hovering over a call's edge marks the lines making it in the caller's card.
  addCallHovers() {
    const container = document.querySelector("#interactive-graph-container");
    const mark = (event, on) => {
      const g = event.target.closest?.("g.edge");
      const call = g && this.calls.get(g.dataset.symbols);
      if (!call) {
        return;
      }
      const card = container.querySelector(`[data-canvas-node="${CSS.escape(call.from)}"]`);
      for (const line of call.lines) {
        card?.querySelector(`.rc-lineno[data-line-number="${line}"]`)
          ?.closest(".source-line-with-number")
          ?.classList.toggle("rc-call-line", on);
      }
      g.classList.toggle("rc-call-hovered", on);
    };
    container.addEventListener("mouseover", event => mark(event, true));
    container.addEventListener("mouseout", event => mark(event, false));
  }

  toggle(field, id) {
    const list = this.state[field];
    const at = list.indexOf(id);
    if (at >= 0) {
      list.splice(at, 1);
    } else {
      list.push(id);
    }
    this.changed();
  }

  changed() {
    this.graph.setInput(this.input());
    this.updateToolbar();
    this.save();
  }

  buildToolbar() {
    const toolbar = document.querySelector("#rc-toolbar");
    const label = document.createElement("label");
    this.contextBox = document.createElement("input");
    this.contextBox.type = "checkbox";
    this.contextBox.className = "rc-context-toggle";
    this.contextBox.addEventListener("change", () => {
      this.state.context = this.contextBox.checked;
      this.changed();
    });
    label.append(this.contextBox, " Callers and callees it doesn't change");

    this.showHidden = this.button("", "Show the cards you hid", () => {
      this.state.hidden = [];
      this.changed();
    });
    this.showHidden.classList.add("rc-show-hidden");
    const collapseAll = this.button("Collapse all", "Show only the functions' names", () => {
      this.state.collapsed = this.model.nodes.map(node => node.id);
      this.changed();
    });
    const expandAll = this.button("Expand all", "Show the functions' code", () => {
      this.state.collapsed = [];
      this.changed();
    });
    const exportButton = this.button("Export", "Save this arrangement as a JSON file, ex: to share it",
                                     () => this.exportState());
    exportButton.classList.add("rc-export");
    const importInput = document.createElement("input");
    importInput.type = "file";
    importInput.accept = "application/json,.json";
    importInput.hidden = true;
    importInput.addEventListener("change", () => this.importState(importInput.files[0]));
    const importButton = this.button("Import…", "Arrange it as a JSON file exported from a canvas says",
                                     () => importInput.click());
    const reset = this.button("Reset", "Forget this arrangement", () => {
      this.state = this.defaultState();
      this.changed();
    });
    this.status = document.createElement("span");
    this.status.className = "rc-status";
    toolbar.append(label, this.showHidden, collapseAll, expandAll, exportButton, importButton,
                   importInput, reset, this.status);

    if (this.model.other_files.length) {
      const others = document.createElement("p");
      others.className = "rc-other-files";
      others.append("Its other files: ");
      this.model.other_files.forEach((file, i) => {
        const a = document.createElement("a");
        a.href = file.link;
        a.textContent = file.path;
        others.append(...(i ? [", "] : []), a, ` (${file.change}, ${file.note})`);
      });
      toolbar.after(others);
    }
  }

  updateToolbar() {
    this.contextBox.checked = this.state.context;
    const hidden = this.state.hidden.length;
    this.showHidden.textContent = `Show ${hidden} hidden`;
    this.showHidden.hidden = hidden == 0;
    this.status.textContent = this.savedAt
      ? `Arranged in this browser ${this.savedAt.slice(0, 16).replace("T", " ")}`
      : "";
  }

  // The arrangement as an export: the push, its model's URL, and the state.
  exported() {
    return {
      format: RevCanvas.FORMAT,
      version: RevCanvas.VERSION,
      tree: this.model.tree,
      push: this.model.push,
      head: this.model.head,
      model: new URL(`/${this.model.tree}/rev-canvas/${this.model.head}/canvas.json`,
                     location.href).href,
      state: this.state,
    };
  }

  exportState() {
    const blob = new Blob([JSON.stringify(this.exported(), null, 2)],
                          { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `canvas-${this.model.tree}-${this.model.push}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  }

  // Apply an export: here, if it's of this push, and otherwise, save it for
  // its push and go to its canvas.
  async importState(file) {
    let imported;
    try {
      imported = JSON.parse(await file.text());
    } catch (e) {
      alert(`That isn't JSON: ${e}`);
      return;
    }
    if (imported?.format != RevCanvas.FORMAT || !imported.tree || !imported.head) {
      alert("That isn't a canvas's arrangement.");
      return;
    }
    if (imported.tree == this.model.tree && imported.head == this.model.head) {
      this.state = this.cleanState(imported.state);
      this.changed();
      return;
    }
    await this.store(`${imported.tree}/${imported.head}`, imported.state);
    location.href = `/${imported.tree}/rev-canvas/${imported.head}`;
  }

  openDb() {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(RevCanvas.DB, 1);
      request.onupgradeneeded = () => request.result.createObjectStore(RevCanvas.STORE);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  }

  async load(key) {
    const db = await this.openDb();
    return new Promise((resolve, reject) => {
      const request = db.transaction(RevCanvas.STORE).objectStore(RevCanvas.STORE).get(key);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  }

  async store(key, state) {
    const db = await this.openDb();
    const saved = new Date().toISOString();
    await new Promise((resolve, reject) => {
      const transaction = db.transaction(RevCanvas.STORE, "readwrite");
      transaction.objectStore(RevCanvas.STORE).put({ state, saved }, key);
      transaction.oncomplete = resolve;
      transaction.onerror = () => reject(transaction.error);
    });
    return saved;
  }

  async save() {
    try {
      this.savedAt = await this.store(this.key, this.state);
      this.updateToolbar();
    } catch (e) {
      console.warn("Couldn't keep the canvas's arrangement:", e);
    }
  }
})();
