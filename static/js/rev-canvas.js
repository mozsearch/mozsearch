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
//
// The push's changes are also sliced into the clusters of dataflow they touch
// (the `slice-rev:` query's JSON, see tools/src/dataflow/slice.rs, loaded after
// the cards): cards are badged with their clusters, which a panel lists grouped
// by their commits and then by class, and one cluster can be shown alone.
// Clusters aren't merged unless asked to: then those of a class (and commits)
// are one.

var RevCanvas = new (class RevCanvas {
  constructor() {
    window.addEventListener("load", () => this.init(), { once: true });
  }

  static FORMAT = "searchfox-rev-canvas";
  static VERSION = 1;
  static DB = "searchfox-rev-canvas";
  static STORE = "arrangements";

  defaultState() {
    return { context: true, hidden: [], collapsed: [], cluster: null, merge: false };
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
    // (Not waited for: the cards are useful without them.)
    this.loadClusters();
    // (Then as this browser last arranged it, if it did.)
    try {
      const saved = await this.load(this.key);
      if (saved) {
        this.state = this.cleanState(saved.state);
        this.savedAt = saved.saved;
        this.graph.setInput(this.input());
        this.updateToolbar();
        if (this.clusters) {
          this.renderClusters();
        }
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
      cluster: typeof state?.cluster == "string" ? state.cluster : null,
      merge: state?.merge === true,
    };
  }

  // The push's slicing (see the file's comment), and its clusters' groups.
  async loadClusters() {
    const query = `slice-rev:${this.model.base}..${this.model.head} slice-format:json`;
    const url = `/${this.model.tree}/query/default?q=${encodeURIComponent(query)}` +
      `&rev=${encodeURIComponent(this.model.head)}`;
    try {
      const response = await fetch(url, { headers: { Accept: "application/json" } });
      if (!response.ok) {
        throw new Error(`${response.status} ${response.statusText}`);
      }
      this.slicing = (await response.json())?.JsonValue?.value;
    } catch (e) {
      console.warn("Couldn't slice the push:", e);
    }
    if (!this.slicing?.clusters) {
      this.clusterNote.textContent = "Its changes couldn't be sliced into dataflow clusters.";
      return;
    }
    this.buildClusters();
    this.renderClusters();
    this.graph.setInput(this.input());
    document.documentElement.dataset.canvasClustersReady = "true";
  }

  // Each cluster (by its number, 1 for the biggest) as its units' functions,
  // classes and commits, and the groups (merged or not) they're shown as.
  buildClusters() {
    const units = this.slicing.units;
    const short = sym => this.nodes.get(sym)?.pretty?.split("::").slice(-2).join("::") ??
      units.find(u => u.func_sym == sym)?.func?.split("::").slice(-2).join("::") ?? sym;
    this.clusters = this.slicing.clusters.map((ids, i) => {
      const members = ids.map(id => units[id]);
      const funcs = [...new Set(members.map(u => u.func_sym).filter(Boolean))];
      const commits = [...new Set(members.flatMap(u => u.commits ?? []))];
      // (Its class: the one most of its functions are methods of, if any.)
      const counts = new Map();
      for (const u of members.filter(u => u.class)) {
        counts.set(u.class, (counts.get(u.class) ?? 0) + 1);
      }
      const cls = [...counts].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
      return {
        n: i + 1,
        funcs,
        names: funcs.map(short),
        units: members.length,
        commits,
        cls,
        // (Which commits and class it's grouped under.)
        group: `${commits.join(",")}|${cls ?? ""}`,
      };
    });
    // A card's clusters, by its function.
    this.cardClusters = new Map();
    for (const cluster of this.clusters) {
      for (const sym of cluster.funcs) {
        if (!this.cardClusters.has(sym)) {
          this.cardClusters.set(sym, []);
        }
        this.cardClusters.get(sym).push(cluster.n);
      }
    }
  }

  // What's shown alone (`state.cluster`): a cluster ("c<n>"), or, merged, all
  // of a group's ("g<group>").
  shownKey(cluster) {
    return this.state.merge ? `g${cluster.group}` : `c${cluster.n}`;
  }

  clusterColor(n) {
    return `hsl(${(n * 137) % 360} 55% 50%)`;
  }

  renderClusters() {
    const list = this.clusterList;
    list.replaceChildren();
    const commitName = commits => commits.length
      ? `${commits.length > 1 ? "Commits" : "Commit"} ${commits.map(c => c.slice(0, 12)).join(", ")}`
      : "Changes";
    const className = cls => cls ? cls.replace(/^T_/, "") : "Not in a class";
    // (By commits, then class, in the clusters' order.)
    const byCommits = new Map();
    for (const cluster of this.clusters) {
      const key = cluster.commits.join(",");
      if (!byCommits.has(key)) {
        byCommits.set(key, new Map());
      }
      const byClass = byCommits.get(key);
      if (!byClass.has(cluster.cls)) {
        byClass.set(cluster.cls, []);
      }
      byClass.get(cluster.cls).push(cluster);
    }
    for (const byClass of byCommits.values()) {
      const commitItem = document.createElement("li");
      commitItem.className = "rc-cluster-commit";
      commitItem.append(commitName([...byClass.values()][0][0].commits));
      const classes = document.createElement("ul");
      for (const [cls, clusters] of byClass) {
        const classItem = document.createElement("li");
        classItem.className = "rc-cluster-class";
        classItem.append(className(cls));
        const items = document.createElement("ul");
        // (Merged: the group as one.)
        const shown = this.state.merge ? [clusters] : clusters.map(c => [c]);
        for (const members of shown) {
          const item = document.createElement("li");
          item.className = "rc-cluster";
          const key = this.shownKey(members[0]);
          const names = members.flatMap(c => c.names);
          const swatch = document.createElement("span");
          swatch.className = "rc-cluster-swatch";
          swatch.style.backgroundColor = this.clusterColor(members[0].n);
          const label = members.map(c => c.n).join("+");
          const show = this.button(
            `${members.length > 1 ? "Clusters" : "Cluster"} ${label}`,
            this.state.cluster == key ? "Show all the cards" : "Show only its cards",
            () => {
              this.state.cluster = this.state.cluster == key ? null : key;
              this.changed();
            });
          show.classList.add("rc-cluster-show");
          show.dataset.cluster = key;
          show.setAttribute("aria-pressed", String(this.state.cluster == key));
          const units = members.reduce((sum, c) => sum + c.units, 0);
          const what = document.createElement("span");
          what.className = "rc-cluster-what";
          what.textContent = `${names.slice(0, 4).join(", ")}${names.length > 4 ? ", …" : ""}` +
            ` (${units} ${units == 1 ? "change" : "changes"})`;
          item.append(swatch, show, " ", what);
          // (Its diagram, for one cluster.)
          if (members.length == 1) {
            const diagram = document.createElement("a");
            diagram.className = "rc-cluster-diagram";
            const query = `slice-rev:${this.model.base}..${this.model.head} slice-cluster:${members[0].n}`;
            diagram.href = `/${this.model.tree}/query/default?q=${encodeURIComponent(query)}` +
              `&rev=${encodeURIComponent(this.model.head)}`;
            diagram.textContent = "diagram";
            item.append(" ", diagram);
          }
          items.append(item);
        }
        classItem.append(items);
        classes.append(classItem);
      }
      commitItem.append(classes);
      list.append(commitItem);
    }
    const other = this.slicing.other?.length ?? 0;
    this.clusterNote.textContent =
      `${this.clusters.length} clusters of the dataflow its changes touch` +
      (other ? `, and ${other} changes without dataflow (ex: includes)` : "") + ".";
    this.mergeBox.checked = this.state.merge;
  }

  // The cards of the cluster (or group) shown alone, if any, and the context's
  // cards calling them or called by them.
  clusterCards() {
    const key = this.state.cluster;
    const members = (this.clusters ?? []).filter(c => this.shownKey(c) == key);
    if (!key || !members.length) {
      return null;
    }
    const cards = new Set(members.flatMap(c => c.funcs).filter(sym => this.nodes.has(sym)));
    for (const edge of this.model.edges) {
      for (const [a, b] of [[edge.from, edge.to], [edge.to, edge.from]]) {
        if (cards.has(a) && this.nodes.get(b)?.status == "context") {
          cards.add(b);
        }
      }
    }
    return cards;
  }

  // The graph's input: the shown cards, and the calls among them.
  input() {
    const hidden = new Set(this.state.hidden);
    const cluster = this.clusterCards();
    const shown = new Set(
      this.model.nodes
        .filter(node => (this.state.context || node.status != "context") && !hidden.has(node.id))
        .filter(node => !cluster || cluster.has(node.id))
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
    head.append(collapse, status, name, where);
    // (Its clusters' badges.)
    for (const n of this.cardClusters?.get(node.id) ?? []) {
      const badge = document.createElement("span");
      badge.className = "rc-cluster-badge";
      badge.textContent = `c${n}`;
      badge.title = `In dataflow cluster ${n}`;
      badge.style.backgroundColor = this.clusterColor(n);
      head.append(badge);
    }
    head.append(hide);
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
    if (this.clusters) {
      this.renderClusters();
    }
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

    // What the cards' edges and the lines' backgrounds mean (with samples of
    // them, styled like them).
    const legend = document.createElement("p");
    legend.className = "rc-legend";
    for (const [sample, text] of [
      ["rc-legend-added", "a function it adds"],
      ["rc-legend-changed", "a function it changes"],
      ["rc-legend-context", "a function it doesn't change, calling or called by those"],
      ["rc-changed-line", "a line it adds or changes"],
      ["rc-call-line", "the lines making a call, while hovering over its arrow"],
    ]) {
      const item = document.createElement("span");
      item.className = "rc-legend-item";
      const swatch = document.createElement("span");
      swatch.className = `rc-legend-sample ${sample}`;
      item.append(swatch, ` ${text}`);
      legend.append(item);
    }
    toolbar.after(legend);

    // The push's dataflow clusters (see `loadClusters`).
    const clusters = document.createElement("details");
    clusters.className = "rc-clusters";
    const summary = document.createElement("summary");
    summary.textContent = "Dataflow clusters";
    this.clusterNote = document.createElement("span");
    this.clusterNote.className = "rc-cluster-note";
    this.clusterNote.textContent = "Slicing its changes…";
    const mergeLabel = document.createElement("label");
    this.mergeBox = document.createElement("input");
    this.mergeBox.type = "checkbox";
    this.mergeBox.className = "rc-cluster-merge";
    this.mergeBox.addEventListener("change", () => {
      this.state.merge = this.mergeBox.checked;
      this.state.cluster = null;
      this.changed();
    });
    mergeLabel.append(this.mergeBox, " Merge clusters of the same class");
    this.clusterList = document.createElement("ul");
    this.clusterList.className = "rc-cluster-list";
    clusters.append(summary, this.clusterNote, " ", mergeLabel, this.clusterList);
    legend.after(clusters);

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
      clusters.after(others);
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
