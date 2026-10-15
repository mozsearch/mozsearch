/**
 * The file timeline: under the search box, in place of the indexed commit's
 * description (which its tooltip keeps), an ambient view of the file's
 * history, from its token-centric blame's commits (BLAME_INFO's: those of its
 * tokens and of the removals they bear the scars of), a tick for each by its
 * time, and a mark for the page's revision.  Clicking it shows those commits,
 * newest first (their headers from the file's commits.json), on a bigger
 * timeline, each tick and item highlighting the other on hover, and for a
 * Firefox tree, the dates of Firefox's releases (product-details', fetched
 * then; each branched from mozilla-central about 4 weeks before).
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
      .map(([rev, time, author], index) => ({ rev, time, author, index }))
      .filter(commit => commit.time > 0)
      .sort((a, b) => a.time - b.time);
    if (!this.commits.length) {
      return;
    }
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

  svg(width, height, { ticks = true, releases = [] } = {}) {
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
    for (const [version, time] of releases) {
      const release = document.createElementNS(NS, "line");
      const x = this.x(time, width);
      release.setAttribute("x1", x);
      release.setAttribute("x2", x);
      release.setAttribute("y1", 0);
      release.setAttribute("y2", height);
      release.classList.add("file-timeline-release");
      const title = document.createElementNS(NS, "title");
      title.textContent = `Firefox ${version}, released ${new Date(time * 1000).toISOString().slice(0, 10)}`;
      release.append(title);
      svg.append(release);
    }
    if (ticks) {
      for (const commit of this.commits) {
        const tick = document.createElementNS(NS, "line");
        const x = this.x(commit.time, width);
        tick.setAttribute("x1", x);
        tick.setAttribute("x2", x);
        tick.setAttribute("y1", height * 0.25);
        tick.setAttribute("y2", height - 1);
        tick.classList.add("file-timeline-tick");
        tick.dataset.commit = commit.index;
        svg.append(tick);
      }
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
      "The file's history: its commits by their dates, and this revision; click for them.";
    button.setAttribute("aria-haspopup", "dialog");
    button.setAttribute("aria-expanded", "false");
    const label = document.createElement("span");
    label.className = "file-timeline-label";
    const first = this.year(this.start);
    const last = this.year(this.commits[this.commits.length - 1].time);
    label.textContent = `${this.commits.length} commit${this.commits.length == 1 ? "" : "s"}, ` +
      (first == last ? `${first}` : `${first}–${last}`);
    button.append(this.svg(300, 14), label);
    button.addEventListener("click", () => this.togglePopup(button));
    this.desc.replaceWith(button);
    this.button = button;
  }

  async releases() {
    const tree = document.getElementById("data")?.dataset.tree || "";
    if (!tree.startsWith("firefox")) {
      return [];
    }
    try {
      const response = await fetch("https://product-details.mozilla.org/1.0/firefox_history_major_releases.json");
      const json = await response.json();
      return Object.entries(json)
        .map(([version, date]) => [version, Date.parse(date) / 1000])
        .filter(([_, time]) => time >= this.start && time <= this.end);
    } catch (e) {
      return [];
    }
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
    const big = document.createElement("div");
    big.className = "file-timeline-big";
    big.append(this.svg(600, 40));
    const list = document.createElement("ol");
    list.className = "file-timeline-commits";
    const close = document.createElement("button");
    close.type = "button";
    close.className = "lens-popup-close";
    close.textContent = "Close";
    close.addEventListener("click", () => this.closePopup());
    popup.append(heading, big, list, close);

    const rect = anchor.getBoundingClientRect();
    popup.style.left = `${Math.max(8, rect.left)}px`;
    popup.style.top = `${rect.bottom + 4}px`;
    document.body.append(popup);
    this.popup = popup;
    anchor.setAttribute("aria-expanded", "true");

    // (Each tick and item highlights the other.)
    const highlight = (index, on) => {
      for (const node of popup.querySelectorAll(`[data-commit="${index}"]`)) {
        node.classList.toggle("file-timeline-highlight", on);
      }
    };
    popup.addEventListener("mouseover", event => {
      const node = event.target.closest?.("[data-commit]");
      if (node) {
        highlight(node.dataset.commit, true);
      }
    });
    popup.addEventListener("mouseout", event => {
      const node = event.target.closest?.("[data-commit]");
      if (node) {
        highlight(node.dataset.commit, false);
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
      item.append(date, " ", link);
      if (this.revTime && Math.abs(commit.time - this.revTime) < 1) {
        item.classList.add("file-timeline-current");
      }
      list.append(item);
    }
    const releases = await this.releases();
    if (releases.length && this.popup === popup) {
      big.replaceChildren(this.svg(600, 40, { releases }));
    }
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
    const tokenMatch = /^#tokens=(\d+)/.exec(hash);
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
