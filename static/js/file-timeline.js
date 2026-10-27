/**
 * The file timeline: under the search box, in place of the indexed commit's
 * description (which its tooltip keeps), an ambient view of the file's
 * history, from its token-centric blame's commits (BLAME_INFO's: those of its
 * tokens and of the removals they bear the scars of), a tick for each by its
 * time, and a mark for the page's revision.  Clicking it shows those commits,
 * newest first (their headers from the file's commits.json), on a bigger
 * timeline, each tick and item highlighting the other on hover (the nearest
 * tick's), and in a band above the ticks, Firefox's version cuts, for a tree
 * with them (its pages/milestones.json, from its config/milestone.txt; see
 * milestone-cuts.py), labeled as they fit, the nearest one's date on hover.
 * Its "since" (a commit or a cut) is the page's baseline: the blame strip
 * highlights the changes since it, and the outline (see outline.js) compares
 * the file with it, through "baselinechange" events.  Its heading has a box
 * searching the file's history (a `text-history:` query of the file; see
 * `historyQuery`).
 */
var FileTimeline = new (class FileTimeline {
  constructor() {
    const info = typeof BLAME_INFO !== "undefined" ? BLAME_INFO : null;
    this.desc = document.getElementById("rev-desc");
    if (!info?.commits?.length || !this.desc) {
      return;
    }
    this.info = info;
    this.commits = info.commits
      .map(([rev, time, author], index) => ({ rev, time, author, index, change: info.changes?.[index] ?? null }))
      .filter(commit => commit.time > 0)
      .sort((a, b) => a.time - b.time);
    if (!this.commits.length) {
      return;
    }
    this.maxChange = Math.max(0, ...this.commits.map(commit => commit.change ?? 0));
    const revTime = document.querySelector("#rev-id time")?.getAttribute("datetime");
    this.revTime = revTime ? Date.parse(revTime) / 1000 : null;
    this.start = this.commits[0].time;
    this.end = Math.max(this.revTime ?? 0, this.commits[this.commits.length - 1].time, Date.now() / 1000);
    this.render();
  }

  x(time, width) {
    return ((time - this.start) / Math.max(1, this.end - this.start)) * width;
  }

  year(time) {
    return new Date(time * 1000).getUTCFullYear();
  }

  /** The UTC day of `time` (in seconds), as YYYY-MM-DD. */
  day(time) {
    return new Date(time * 1000).toISOString().slice(0, 10);
  }

  /**
   * How tall a commit's tick is, as a fraction of the timeline's height: by
   * the logarithm of how much the commit changed the file, from 20% for the
   * least change to 80% for the most (the top 20% being the cuts' band), or
   * 80% without the sizes of the changes.
   */
  tickHeight(commit) {
    if (!this.maxChange || commit.change == null) {
      return 0.8;
    }
    return 0.2 + 0.6 * Math.log1p(commit.change) / Math.log1p(this.maxChange);
  }

  svg(width, height, { cuts = [] } = {}) {
    const NS = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(NS, "svg");
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.setAttribute("preserveAspectRatio", "none");
    svg.classList.add("file-timeline-svg");
    const line = document.createElementNS(NS, "line");
    line.setAttribute("x1", 0);
    line.setAttribute("x2", width);
    line.setAttribute("y1", height - 1);
    line.setAttribute("y2", height - 1);
    line.classList.add("file-timeline-axis");
    svg.append(line);
    // (In the top 20%, above the ticks.)
    cuts.forEach((cut, i) => {
      const release = document.createElementNS(NS, "line");
      const x = this.x(cut.time, width);
      release.setAttribute("x1", x);
      release.setAttribute("x2", x);
      release.setAttribute("y1", 0);
      release.setAttribute("y2", height * 0.2);
      release.classList.add("file-timeline-release");
      release.dataset.cut = i;
      svg.append(release);
    });
    for (const commit of this.commits) {
      const tick = document.createElementNS(NS, "line");
      const x = this.x(commit.time, width);
      tick.setAttribute("x1", x);
      tick.setAttribute("x2", x);
      tick.setAttribute("y1", (height - 1) * (1 - this.tickHeight(commit)));
      tick.setAttribute("y2", height - 1);
      tick.classList.add("file-timeline-tick");
      tick.dataset.commit = commit.index;
      svg.append(tick);
    }
    if (this.revTime) {
      const mark = document.createElementNS(NS, "circle");
      mark.setAttribute("cx", this.x(this.revTime, width));
      mark.setAttribute("cy", height / 2);
      mark.setAttribute("r", Math.max(2, height / 5));
      mark.classList.add("file-timeline-rev");
      svg.append(mark);
    }
    return svg;
  }

  render() {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "file-timeline";
    button.title = `${this.desc.title || this.desc.textContent}\n\n` +
      "The file's history: its commits by their dates" +
      (this.maxChange ? " (taller for bigger changes to the file)" : "") +
      ", and this revision; click for them.";
    button.setAttribute("aria-haspopup", "dialog");
    button.setAttribute("aria-expanded", "false");
    const label = document.createElement("span");
    label.className = "file-timeline-label";
    const first = this.year(this.start);
    const last = this.year(this.commits[this.commits.length - 1].time);
    label.textContent = `${this.commits.length} commit${this.commits.length == 1 ? "" : "s"}, ` +
      (first == last ? `${first}` : `${first}–${last}`);
    button.append(this.svg(600, 14), label);
    button.addEventListener("click", () => this.togglePopup(button));
    this.desc.replaceWith(button);
    this.button = button;
  }

  /** The page's file's path (of a source listing or a revision's page). */
  path() {
    const match = document.location.pathname.match(/^\/[^\/]+\/(?:source|rev\/[^\/]+)\/(.*)$/);
    return match ? match[1] : "";
  }

  /** The page's revision (a revision's page's, or the indexed one). */
  pageRev() {
    return document.location.pathname.match(/^\/[^\/]+\/rev\/([0-9a-f]{7,40})\//i)?.[1] ??
      document.querySelector("#rev-id a")?.getAttribute("href")?.split("/").pop();
  }

  /** The page of the file's outline diff from `from` (see outline.js) to the page's revision. */
  outlineDiffURL(from, to = this.pageRev()) {
    const tree = document.getElementById("data")?.dataset.tree;
    return `/${tree}/outline-diff/${encodeURIComponent(from)}/${to}/${this.path()}`;
  }

  /**
   * The tree's Firefox version cuts (see milestone-cuts.py), oldest first:
   * each version's cut commit (the bump to the next version), its parent (the
   * version's last commit), and its time.
   */
  allCuts() {
    this.cutsPromise ??= (async () => {
      const tree = document.getElementById("data")?.dataset.tree;
      try {
        const response = await fetch(`/${tree}/pages/milestones.json`);
        if (!response.ok) {
          return [];
        }
        return (await response.json()).map(([version, rev, parent, time]) => ({ version, rev, parent, time }));
      } catch (e) {
        return [];
      }
    })();
    return this.cutsPromise;
  }

  async cuts() {
    return (await this.allCuts()).filter(cut => cut.time >= this.start && cut.time <= this.end);
  }

  /**
   * The cuts to label, on a timeline `width` pixels wide: those of every
   * Nth version, N the smallest of 1, 2, 5, 10... that keeps their labels
   * `gap` pixels apart.
   */
  labeledCuts(cuts, width, gap = 32) {
    for (const step of [1, 2, 5, 10, 20, 50, 100]) {
      const labeled = cuts.filter(cut => cut.version % step == 0);
      const xs = labeled.map(cut => this.x(cut.time, width));
      if (xs.every((x, i) => i == 0 || x - xs[i - 1] >= gap)) {
        return labeled;
      }
    }
    return [];
  }

  /** The cut or commit nearest the time `time`, of those within `within` seconds. */
  nearest(items, time, within = Infinity) {
    let best = null;
    for (const item of items) {
      const distance = Math.abs(item.time - time);
      if (distance <= within && (!best || distance < Math.abs(best.time - time))) {
        best = item;
      }
    }
    return best;
  }

  /**
   * The query searching the file's history for `text` (the query language's
   * `text-history:`, see `history-grep`), in the page's file: `path:`,
   * anchored, or if the path has what globs or regular expressions take for
   * syntax (ex: `+`, `[`), `pathre:`, escaped.  (Values with spaces, quotes or
   * colons are quoted, as `Foo::Bar` would be the term `Foo`.)
   */
  historyQuery(text) {
    text = text.trim();
    const path = document.getElementById("data")?.dataset.path;
    if (!text || !path) {
      return null;
    }
    const quote = value => /[\s'"\\:]/.test(value)
      ? `'${value.replace(/[\\']/g, c => `\\${c}`)}'`
      : value;
    const scope = /[*?{}[\]+^$\\]/.test(path)
      ? `pathre:${quote(`^${path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`)}`
      : `path:${quote(`^${path}$`)}`;
    return `text-history:${quote(text)} ${scope}`;
  }

  /**
   * Set the page's baseline (or clear it, with null): `time`, for the blame
   * strip, and `from`, the revision (or "since:YYYY-MM-DD") for the outline,
   * with its `label`.
   */
  setBaseline(baseline) {
    if (baseline) {
      BlameColorizer.applySince(baseline.time);
    } else {
      BlameColorizer.apply(Settings.blame.colorMode);
    }
    this.baseline = baseline;
    document.dispatchEvent(new CustomEvent("baselinechange", { detail: baseline }));
  }

  async togglePopup(anchor) {
    if (this.popup) {
      this.closePopup();
      return;
    }
    const popup = document.createElement("div");
    popup.className = "lens-popup file-timeline-popup";
    popup.setAttribute("role", "dialog");
    popup.setAttribute("aria-label", "The file's history");
    const heading = document.createElement("h5");
    heading.textContent = "The file's history";
    // (Beside it, a search of the file's history; see `historyQuery`.)
    const header = document.createElement("div");
    header.className = "file-timeline-header";
    const search = document.createElement("form");
    search.className = "file-timeline-search";
    search.setAttribute("role", "search");
    const input = document.createElement("input");
    input.type = "search";
    input.placeholder = "Search this file's history";
    input.setAttribute("aria-label", "Search this file's history");
    search.append(input);
    search.addEventListener("submit", event => {
      event.preventDefault();
      const query = this.historyQuery(input.value);
      const tree = document.getElementById("data")?.dataset.tree;
      if (query && tree) {
        location.href = queryUrl(tree, query);
      }
    });
    header.append(heading, search);
    const big = document.createElement("div");
    big.className = "file-timeline-big";
    // (The cuts' labels, the timeline, and what the pointer's nearest to.)
    const labels = document.createElement("div");
    labels.className = "file-timeline-versions";
    const readout = document.createElement("div");
    readout.className = "file-timeline-readout";
    big.append(labels, this.svg(600, 40), readout);
    let cuts = [];
    const list = document.createElement("ol");
    list.className = "file-timeline-commits";
    // (Highlighting the changes since a commit or a cut on the strip.)
    const since = document.createElement("div");
    since.className = "file-timeline-since";
    const hint = () => {
      since.textContent = cuts.length
        ? "Click a version in the band above the commits, or a commit’s “since”, to highlight the changes since it."
        : "Click a commit’s “since” to highlight the changes since it.";
    };
    const showSince = (baseline, apply = true) => {
      since.replaceChildren();
      if (apply) {
        this.setBaseline(baseline);
      }
      if (!baseline) {
        hint();
        return;
      }
      const clear = document.createElement("button");
      clear.type = "button";
      clear.textContent = "Clear";
      clear.addEventListener("click", () => showSince(null));
      const compare = document.createElement("a");
      compare.href = this.outlineDiffURL(baseline.from);
      compare.textContent = "Compare the outlines";
      since.append(`Highlighting the changes since ${baseline.label}. `, compare, " ", clear);
    };
    const close = document.createElement("button");
    close.type = "button";
    close.className = "lens-popup-close";
    close.textContent = "Close";
    close.addEventListener("click", () => this.closePopup());
    popup.append(header, big, since, list, close);
    if (this.baseline) {
      showSince(this.baseline, false);
    } else {
      hint();
    }

    // (At the page's indent, where "Showing" starts, under the timeline.)
    const rect = anchor.getBoundingClientRect();
    const indent = document.getElementById("rev-id")?.getBoundingClientRect().left ?? rect.left;
    popup.style.left = `${Math.max(8, indent)}px`;
    popup.style.top = `${rect.bottom + 4}px`;
    document.body.append(popup);
    this.popup = popup;
    anchor.setAttribute("aria-expanded", "true");

    // (Each item highlights its tick, and the pointer on the timeline the
    // nearest cut, in the band, or else the nearest tick and its item.)
    const highlight = (index, on) => {
      for (const node of popup.querySelectorAll(`[data-commit="${index}"]`)) {
        node.classList.toggle("file-timeline-highlight", on);
      }
    };
    popup.addEventListener("mouseover", event => {
      const node = event.target.closest?.("li[data-commit]");
      if (node) {
        highlight(node.dataset.commit, true);
      }
    });
    popup.addEventListener("mouseout", event => {
      const node = event.target.closest?.("li[data-commit]");
      if (node) {
        highlight(node.dataset.commit, false);
      }
    });
    let hovered = null;
    const unhover = () => {
      if (hovered) {
        for (const node of big.querySelectorAll(".file-timeline-highlight")) {
          node.classList.remove("file-timeline-highlight");
        }
        highlight(hovered.index, false);
      }
      hovered = null;
      readout.textContent = "";
    };
    const pointed = event => {
      const svg = big.querySelector("svg");
      const box = svg.getBoundingClientRect();
      const time = this.start + ((event.clientX - box.left) / box.width) * (this.end - this.start);
      // (Seconds per pixel, for snapping to ticks.)
      const pixel = (this.end - this.start) / box.width;
      const inBand = event.clientY < box.top + box.height * 0.2;
      if (cuts.length && inBand) {
        const cut = this.nearest(cuts, time);
        return cut && { cut };
      }
      const commit = this.nearest(this.commits, time, 6 * pixel);
      return commit && { commit };
    };
    big.addEventListener("pointermove", event => {
      const target = pointed(event);
      if (target?.cut === hovered?.cut && target?.commit === hovered?.commit) {
        return;
      }
      unhover();
      if (target?.cut) {
        const i = cuts.indexOf(target.cut);
        for (const node of big.querySelectorAll(`[data-cut="${i}"]`)) {
          node.classList.add("file-timeline-highlight");
        }
        readout.textContent = `Firefox ${target.cut.version}: cut from mozilla-central on ${this.day(target.cut.time)}` +
          " (click for the changes since)";
        hovered = { cut: target.cut };
      } else if (target?.commit) {
        highlight(target.commit.index, true);
        hovered = { commit: target.commit, index: target.commit.index };
      }
    });
    big.addEventListener("pointerleave", unhover);
    big.addEventListener("click", event => {
      const target = pointed(event);
      if (target?.cut) {
        showSince({ time: target.cut.time, from: target.cut.parent, label: `Firefox ${target.cut.version}` });
      } else if (target?.commit) {
        list.querySelector(`li[data-commit="${target.commit.index}"]`)?.scrollIntoView({ block: "nearest" });
      }
    });

    this.dismiss = event => {
      if (event.type === "keydown" ? event.key === "Escape"
          : !popup.contains(event.target) && !anchor.contains(event.target)) {
        this.closePopup();
      }
    };
    document.addEventListener("keydown", this.dismiss);
    document.addEventListener("pointerdown", this.dismiss, true);

    const tree = document.getElementById("data")?.dataset.tree;
    let headers = [];
    if (this.info.dataUrl) {
      try {
        headers = await (await fetch(`${this.info.dataUrl}/commits.json`)).json();
      } catch (e) {
        headers = [];
      }
    }
    if (this.popup !== popup) {
      return;
    }
    // (With a column of the commits' most changed definitions, if we know
    // them.)
    popup.classList.toggle("file-timeline-with-changes", headers.some(info => info?.changes?.length));
    for (const commit of [...this.commits].reverse()) {
      const item = document.createElement("li");
      item.dataset.commit = commit.index;
      const date = document.createElement("time");
      date.textContent = new Date(commit.time * 1000).toISOString().slice(0, 10);
      const link = document.createElement("a");
      link.href = `/${tree}/commit/${commit.rev}`;
      // (The header's summary's text, without its author line or links.)
      const header = document.createElement("template");
      header.innerHTML = (headers[commit.index]?.header ?? commit.rev.slice(0, 12)).split("<br>")[0];
      link.textContent = header.content.textContent;
      // (The commit's diff of the file, the file then, how its outline
      // changed, and highlighting the changes since it.)
      const path = this.path();
      const parent = headers[commit.index]?.parent ?? `${commit.rev}^`;
      const extras = document.createElement("span");
      extras.className = "file-timeline-extras";
      for (const [text, href] of [["diff", `/${tree}/diff/${commit.rev}/${path}`],
                                  ["file", `/${tree}/rev/${commit.rev}/${path}`],
                                  ["outline", this.outlineDiffURL(parent, commit.rev)]]) {
        const a = document.createElement("a");
        a.href = href;
        a.textContent = text;
        extras.append(a, " ");
      }
      const sinceButton = document.createElement("button");
      sinceButton.type = "button";
      sinceButton.textContent = "since";
      sinceButton.title = "Highlight the changes since this commit on the blame strip";
      // (Since the commit, so from its parent.)
      sinceButton.addEventListener("click", () =>
        showSince({ time: commit.time, from: parent, label: date.textContent }));
      extras.append(sinceButton);
      const summary = document.createElement("div");
      summary.className = "file-timeline-summary";
      summary.append(date, " ", link, " ", extras);
      item.append(summary, this.changesCell(headers[commit.index]?.changes ?? []));
      if (this.revTime && Math.abs(commit.time - this.revTime) < 1) {
        item.classList.add("file-timeline-current");
      }
      list.append(item);
    }
    cuts = await this.cuts();
    if (cuts.length && this.popup === popup) {
      big.querySelector("svg").replaceWith(this.svg(600, 40, { cuts }));
      const width = big.getBoundingClientRect().width;
      for (const cut of this.labeledCuts(cuts, width)) {
        const label = document.createElement("span");
        label.dataset.cut = cuts.indexOf(cut);
        label.style.left = `${(this.x(cut.time, 1) * 100).toFixed(3)}%`;
        label.textContent = cut.version;
        labels.append(label);
      }
      if (!this.baseline) {
        hint();
      }
    }
  }

  /**
   * A commit's most changed definitions in the file (its `commits.json`
   * entry's `changes`; see `hyperblame::file_changes`), one per line.
   */
  changesCell(changes) {
    const cell = document.createElement("div");
    cell.className = "file-timeline-changes";
    // (The last two parts of a qualified name, for the column's width.)
    const short = pretty => pretty == "%"
      ? "(top level)"
      : pretty.split("::").slice(-2).map(contextDisplayName).join("::");
    const file = path => path.split("/").pop();
    for (const change of changes) {
      const line = document.createElement("div");
      line.className = `file-timeline-change file-timeline-change-${change.kind}`;
      const name = document.createElement("span");
      name.className = "file-timeline-change-name";
      name.textContent = short(change.pretty);
      const what = {
        added: "added",
        removed: "removed",
        changed: "changed",
        renamed: `renamed from ${contextDisplayName(change.other)}`,
        "moved-in": `moved from ${change.other}`,
        "moved-out": `moved to ${change.other}`,
      }[change.kind] ?? change.kind;
      line.title = `${change.pretty == "%" ? "Outside of any definition" : contextDisplayName(change.pretty)}: ${what}` +
        ` (${change.added} tokens added, ${change.removed} removed)`;
      const sign = { added: "+ ", removed: "\u2212 " }[change.kind] ?? "";
      const after = {
        renamed: ` \u2190 ${short(change.other ?? "")}`,
        "moved-in": ` \u2190 ${file(change.other ?? "")}`,
        "moved-out": ` \u2192 ${file(change.other ?? "")}`,
      }[change.kind] ?? "";
      const tokens = document.createElement("span");
      tokens.className = "file-timeline-change-tokens";
      tokens.textContent = [change.added && `+${change.added}`, change.removed && `\u2212${change.removed}`]
        .filter(Boolean).join(" ");
      line.append(sign, name, after, " ", tokens);
      cell.append(line);
    }
    return cell;
  }

  closePopup() {
    this.popup?.remove();
    this.popup = null;
    this.button?.setAttribute("aria-expanded", "false");
    document.removeEventListener("keydown", this.dismiss);
    document.removeEventListener("pointerdown", this.dismiss, true);
  }
})();

/**
 * The note on a source listing that a permalink led to because the file is
 * the same at the permalink's revision as at the indexed one (see the web
 * server's `/rev/` route): it says so, with a link to the file at exactly
 * that revision, and the URL goes back to the source listing's.
 */
var PermalinkNote = new (class PermalinkNote {
  constructor() {
    const params = new URLSearchParams(document.location.search);
    const rev = params.get("permalink");
    const match = document.location.pathname.match(/^\/([^\/]+)\/source\/(.*)$/);
    if (!rev || !/^[0-9a-f]{7,40}$/i.test(rev) || !match) {
      return;
    }
    const [, tree, path] = match;
    const note = document.createElement("section");
    note.className = "info-box info-box-info info-box-concise permalink-note";
    const heading = document.createElement("h4");
    heading.textContent = "Permalink";
    const text = document.createElement("span");
    text.className = "info-box-facts";
    text.append(`The file is the same at ${rev.slice(0, 12)} as in the indexed revision, so this is its current view.`);
    const exact = document.createElement("a");
    exact.href = `/${tree}/rev/${rev}/${path}?exactly${document.location.hash}`;
    exact.className = "info-box-details";
    exact.textContent = `Show it at ${rev.slice(0, 12)}`;
    note.append(heading, text, exact);
    const boxes = document.getElementById("info-boxes-container");
    const file = document.getElementById("file");
    if (boxes) {
      boxes.prepend(note);
    } else if (file) {
      file.before(note);
    } else {
      document.getElementById("content")?.prepend(note);
    }
    params.delete("permalink");
    const search = params.toString();
    window.history.replaceState(window.history.state, "",
      document.location.pathname + (search ? `?${search}` : "") + document.location.hash);
  }
})();

/**
 * On a page of a file at a revision at which it differs from the indexed one
 * (a permalink's, see the web server's `/rev/` route), with lines or tokens
 * selected: a note with "Where is this now?", which follows the selection's
 * first token into the future (see `hyperblame::future`), to where it is now,
 * or the commit which removed it.
 */
var PermalinkFuture = new (class PermalinkFuture {
  constructor() {
    const info = typeof BLAME_INFO !== "undefined" ? BLAME_INFO : null;
    const match = document.location.pathname.match(/^\/([^\/]+)\/rev\/([0-9a-f]{7,40})\/(.+)$/i);
    if (!info?.peepholeUrl || !info.dataUrl?.includes("/rev-hyperblame/") || !match) {
      return;
    }
    const [, tree, rev] = match;
    // (Not for exactly the revision, which may be the same file as now; see
    // PermalinkNote.)
    if (new URLSearchParams(document.location.search).has("exactly")) {
      return;
    }
    const hash = document.location.hash;
    // (A permalink's selected token, see panel.js's permalinkURL, or the
    // hash's.)
    const tokenParam = new URLSearchParams(document.location.search).get("token");
    const tokenMatch = /^\d+$/.test(tokenParam || "") ? [null, tokenParam] : /^#tokens=(\d+)/.exec(hash);
    const lineMatch = /^#(\d+)/.exec(hash);
    if (!tokenMatch && !lineMatch) {
      return;
    }
    this.tree = tree;
    this.info = info;

    const note = document.createElement("section");
    note.className = "info-box info-box-info info-box-concise permalink-note";
    const heading = document.createElement("h4");
    heading.textContent = "Permalink";
    const text = document.createElement("span");
    text.className = "info-box-facts";
    text.textContent = `This is the file at ${rev.slice(0, 12)}, which has changed since.`;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "info-box-details";
    button.textContent = "Where is this now?";
    const status = document.createElement("span");
    status.className = "permalink-future";
    note.append(heading, text, status, button);
    const boxes = document.getElementById("info-boxes-container");
    const file = document.getElementById("file");
    if (boxes) {
      boxes.prepend(note);
    } else if (file) {
      file.before(note);
    } else {
      return;
    }

    button.addEventListener("click", async () => {
      button.disabled = true;
      status.textContent = "Following it…";
      try {
        let token = tokenMatch ? parseInt(tokenMatch[1], 10) : null;
        if (token === null) {
          const tokens = await HyperblameContexts.contexts[0].getLineTokens(parseInt(lineMatch[1], 10));
          token = tokens?.[0]?.index;
        }
        if (!token) {
          status.textContent = "The line has no tokens to follow.";
          return;
        }
        const url = info.peepholeUrl.replace(/\/peephole$/, "/future");
        const { future, commits } = await (await fetch(`${url}/${token}.json`)).json();
        status.replaceChildren();
        if (future.outcome == "now") {
          const link = document.createElement("a");
          link.href = `/${tree}/source/${future.path.split("/").map(encodeURIComponent).join("/")}#tokens=${future.token}`;
          link.textContent = future.changes.length
            ? `It's now here, after ${future.changes.length} change${future.changes.length == 1 ? "" : "s"}`
            : "It's now here";
          status.append(link);
        } else if (future.outcome == "removed" || future.outcome == "deleted") {
          const header = document.createElement("template");
          header.innerHTML = (commits[future.rev]?.header || future.rev.slice(0, 12)).split("<br>")[0];
          const link = document.createElement("a");
          link.href = `/${tree}/commit/${future.rev}`;
          link.textContent = header.content.textContent;
          status.append(future.outcome == "removed" ? "It was removed by " : "Its file was deleted by ", link);
        } else {
          status.textContent = "We couldn't follow it further.";
        }
      } catch (e) {
        status.textContent = "We couldn't follow it.";
      }
    });
  }
})();
