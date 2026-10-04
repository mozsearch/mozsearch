/**
 * The token-centric blame popup (see "Token blame UI plan" in the hyperblame
 * notes).  Pages with token-centric blame have `BLAME_INFO` (see
 * `page_blame.rs`; diffs have one for each version of the file, see
 * `HyperblameContexts`), which names the page's "hyperblame" data files: what
 * the popup shows about each commit (`commits.json`), and the tokens of the
 * lines in chunks (`lines-K.json`, see `LinesChunk`).
 */

/**
 * Loads the hyperblame data files of one of the page's token-centric blames
 * (see `HyperblameContexts`), starting with the commits and the chunk with the
 * page's first selected line (or its first line), and loading other chunks as
 * they near the viewport or when they're needed.
 */
class HyperblameData {
  /**
   * `rows` has the blame's rows by (1-based) line on diffs, whose rows' ids
   * are only for the lines of the file in the commit.
   */
  constructor(info, rows) {
    this.info = info;
    this.rows = rows;
    this.available = !!info.dataUrl;
    if (!this.available) {
      return;
    }
    this.commitsPromise = null;
    // Chunk promises by chunk index.
    this.chunkPromises = new Map();

    // The (1-based) index of each line's first token.
    this.lineFirstTokens = [];
    let total = 0;
    for (const count of info.tokenCounts) {
      this.lineFirstTokens.push(total + 1);
      total += count;
    }

    this.getCommits();
    this.getChunk(this.chunkForLine(this.firstLine()));

    // Load chunks when their first line (or on diffs, the first of their lines
    // which the diff has) gets near the viewport.
    if (info.chunks.length > 1) {
      const observer = new IntersectionObserver(entries => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            observer.unobserve(entry.target);
            this.getChunk(parseInt(entry.target.dataset.hyperblameChunk, 10));
          }
        }
      }, { rootMargin: "100% 0px" });
      info.chunks.forEach((firstLine, k) => {
        const line = this.rowAtOrAfter(firstLine + 1, info.chunks[k + 1] ?? Infinity);
        if (line) {
          line.dataset.hyperblameChunk = k;
          observer.observe(line);
        }
      });
    }
  }

  /**
   * The row of the given (1-based) line, if the page has it.
   */
  rowFor(lineno) {
    return this.rows ? this.rows.get(lineno) : document.getElementById(`line-${lineno}`);
  }

  stripFor(lineno) {
    return this.rowFor(lineno)?.querySelector(".blame-strip");
  }

  /**
   * The row of the first line from `lineno` up to (but not including)
   * `before` which the page has.
   */
  rowAtOrAfter(lineno, before) {
    if (!this.rows) {
      return this.rowFor(lineno);
    }
    let first = null;
    for (const line of this.rows.keys()) {
      if (line >= lineno && line < before && (first === null || line < first)) {
        first = line;
      }
    }
    return first === null ? null : this.rows.get(first);
  }

  /**
   * The (1-based) line of the page's first selected line, if it's one of
   * ours, or else of our first line.
   */
  firstLine() {
    const selected = document.querySelector(".source-line-with-number.highlighted");
    if (!this.rows) {
      return selected ? parseInt(selected.id.substring("line-".length), 10) : 1;
    }
    for (const [line, row] of this.rows) {
      if (row === selected) {
        return line;
      }
    }
    return this.rows.size ? Math.min(...this.rows.keys()) : 1;
  }

  getCommits() {
    if (!this.commitsPromise) {
      this.commitsPromise = fetch(`${this.info.dataUrl}/commits.json`).then(r => r.json());
    }
    return this.commitsPromise;
  }

  /**
   * The index of the chunk with the given (1-based) line.
   */
  chunkForLine(lineno) {
    const chunks = this.info.chunks;
    let k = 0;
    while (k + 1 < chunks.length && chunks[k + 1] < lineno) {
      k++;
    }
    return k;
  }

  getChunk(k) {
    if (!this.chunkPromises.has(k)) {
      this.chunkPromises.set(
        k,
        fetch(`${this.info.dataUrl}/lines-${k}.json`)
          .then(r => r.json())
          .then(chunk => this.decodeChunk(chunk))
      );
    }
    return this.chunkPromises.get(k);
  }

  /**
   * Decode a chunk's compact token descriptions (see `LinesChunk` in
   * `page_blame.rs`) into `{ start, end, commit, path, lineno, index, pred }`
   * objects, where `start` and `end` are UTF-16 offsets in the line, `lineno`
   * is the token's index in the file in `commit`, and `index` is its index in
   * this file.
   */
  decodeChunk(chunk) {
    const lastLinenos = new Map();
    const lines = chunk.lines.map((tokens, i) => {
      let column = 0;
      let index = this.lineFirstTokens[chunk.firstLine + i];
      return tokens.map(([gap, length, commit, linenoDelta, path = 0]) => {
        const key = `${commit}:${path}`;
        const lineno = (lastLinenos.get(key) || 0) + linenoDelta;
        lastLinenos.set(key, lineno);
        const start = column + gap;
        column = start + length;
        const token = { start, end: column, commit, path, lineno, index };
        const pred = chunk.preds[index];
        if (pred) {
          token.pred = { commit: pred[0], path: pred[1], lineno: pred[2] };
        }
        index++;
        return token;
      });
    });
    return { firstLine: chunk.firstLine, lines };
  }

  /**
   * The tokens of the given (1-based) line.
   */
  async getLineTokens(lineno) {
    const chunk = await this.getChunk(this.chunkForLine(lineno));
    return chunk.lines[lineno - 1 - chunk.firstLine];
  }
}

/**
 * The page's token-centric blames: a source listing has one, of its file
 * (`BLAME_INFO`), and a diff has the blame of the file in the commit and then
 * in each of the commit's parents (`BLAME_INFOS`, with null for those which
 * don't have the file or its blame).  A diff's strips name their blame (its
 * index in `BLAME_INFOS`) and line in that file with `data-hb-ctx` and
 * `data-hb-line`; see `format_diff` in `format.rs`.
 */
var HyperblameContexts = new (class HyperblameContexts {
  constructor() {
    const isDiff = typeof BLAME_INFOS !== "undefined";
    const infos = isDiff ? BLAME_INFOS : typeof BLAME_INFO !== "undefined" ? [BLAME_INFO] : [];
    const rows = infos.map(() => new Map());
    if (isDiff) {
      for (const strip of document.querySelectorAll(".blame-strip[data-hb-ctx]")) {
        rows[strip.dataset.hbCtx].set(parseInt(strip.dataset.hbLine, 10),
                                      strip.closest(".source-line-with-number"));
      }
    }
    this.contexts = infos.map((info, i) => info && new HyperblameData(info, isDiff ? rows[i] : null));
  }

  /**
   * The blame (a `HyperblameData`) of a strip element and its (1-based) line
   * in that blame's file.
   */
  forStrip(elt) {
    if (elt.dataset.hbCtx !== undefined) {
      return { data: this.contexts[elt.dataset.hbCtx], lineno: parseInt(elt.dataset.hbLine, 10) };
    }
    const lineElt = elt.closest(".source-line-with-number");
    return { data: this.contexts[0], lineno: parseInt(lineElt.id.substring("line-".length), 10) };
  }
})();

/**
 * Renders the token-centric blame popup for a line into `BlamePopup.popup`:
 *
 * - A replica of the line, positioned over the line itself, with a span for
 *   each token.
 * - Above it, a row for each commit which introduced some of the line's
 *   tokens, from the newest (nearest the line) to the oldest, with a lane from
 *   each token up to a dot on its commit's row.
 * - Below it, the details of the commits in the same order, followed by the
 *   removals of tokens within and around the line.
 *
 * Hovering over a token, row, or commit's details highlights the commit's
 * tokens, lanes, row, and details.  Clicking on a token shows links for it.
 */
var TokenBlamePopup = new (class TokenBlamePopup {
  get tree() {
    return document.getElementById("data").getAttribute("data-tree");
  }

  encodePath(path) {
    return path.split("/").map(encodeURIComponent).join("/");
  }

  revLink(rev, path, tokens) {
    const hash = tokens ? `#tokens=${tokens}` : "";
    return `/${this.tree}/rev/${rev}/${this.encodePath(path)}${hash}`;
  }

  relativeDate(time) {
    const days = (Date.now() / 1000 - time) / 86400;
    if (days < 1) {
      return "today";
    }
    for (const [unit, length] of [["year", 365], ["month", 30], ["week", 7], ["day", 1]]) {
      if (days >= length) {
        const n = Math.floor(days / length);
        return `${n} ${unit}${n > 1 ? "s" : ""} ago`;
      }
    }
  }

  /**
   * Render the popup for the strip element `elt` into `popup`, returning false
   * if the page doesn't have the data (or `elt` is no longer the trigger).
   */
  async render(popup, elt) {
    const { data, lineno } = HyperblameContexts.forStrip(elt);
    if (!data?.available) {
      return false;
    }
    const lineElt = elt.closest(".source-line-with-number");
    let commits, tokens;
    try {
      [commits, tokens] = await Promise.all([
        data.getCommits(),
        data.getLineTokens(lineno),
      ]);
    } catch (ex) {
      // The popup without the data will do.
      console.error("Couldn't load the hyperblame data:", ex);
      return false;
    }
    if (BlamePopup.triggerElement != elt) {
      return true;
    }
    // The blame the popup is for, which the methods below use.
    this.data = data;
    this.info = data.info;

    // The line's commits, newest first, which get distinct colors.
    const lineCommits = [...new Set(tokens.map(t => t.commit))];
    lineCommits.sort((a, b) => this.info.commits[b][1] - this.info.commits[a][1]);
    this.colors = new Map(lineCommits.map((commit, i) => [commit, `var(--hb-lane-${i % 8})`]));

    const root = document.createElement("div");
    root.className = "hb-popup";

    const upper = document.createElement("div");
    upper.className = "hb-upper";
    // Oldest at the top.
    for (const commit of [...lineCommits].reverse()) {
      upper.append(this.renderRow(commit, commits[commit]));
    }

    const replica = document.createElement("div");
    replica.className = "hb-replica";
    // Clicking on a token shows our menu; see `showTokenMenu`.
    replica.dataset.ownContextMenu = "";
    replica.append(this.renderReplica(lineElt.querySelector("code.source-line"), tokens));

    const details = document.createElement("div");
    details.className = "hb-details";
    if (!tokens.length) {
      details.append(this.renderNote(elt, commits));
    }
    for (const commit of lineCommits) {
      details.append(this.renderDetails(commit, commits[commit], tokens));
    }
    for (const removal of this.lineRemovals(elt, lineno)) {
      details.append(this.renderRemoval(removal, commits[removal.commit]));
    }

    const lanes = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    lanes.classList.add("hb-lanes");
    root.append(lanes, upper, replica, details);

    popup.innerHTML = "";
    popup.classList.add("hb-popup-host");
    popup.append(root);
    popup.style.display = "";

    this.current = { popup, elt, lineElt, root, upper, replica, lanes, tokens };
    this.position();
    this.drawLanes(root, lanes, upper, replica);
    this.bindInteractions(root, tokens);
    return true;
  }

  /**
   * Position the popup so the replica's text is over the line's text, starting
   * the popup at the strip's right edge so the mouse can move from the strip
   * into it.
   */
  position() {
    const { popup, elt, lineElt, replica } = this.current;
    const stripRect = elt.getBoundingClientRect();
    const codeRect = lineElt.querySelector("code.source-line").getBoundingClientRect();
    replica.style.paddingLeft = `${codeRect.left - stripRect.right}px`;
    const lineRect = lineElt.getBoundingClientRect();
    const left = stripRect.right + window.scrollX;
    let top = lineRect.top + window.scrollY - replica.offsetTop;
    // Don't let the popup start above the page.
    top = Math.max(top, window.scrollY);
    popup.style.transform = `translatey(${top}px) translatex(${left}px)`;
  }

  /**
   * Replace the commits' rows above the line with a panel for following
   * tokens (`hb-history`), with a status line and a list of `hb-step`s, and
   * mark the replica's tokens which we're following.
   */
  startFollowing(followed, statusText) {
    const { root, upper, lanes, replica } = this.current;
    // Keep the popup until the user clicks elsewhere.
    BlameStripHoverHandler.keepVisible = true;
    root.classList.add("hb-history-mode");
    // Stop highlighting whatever the user clicked on.
    root.classList.remove("hb-has-hot");
    for (const elt of root.querySelectorAll(".hb-hot")) {
      elt.classList.remove("hb-hot");
    }
    lanes.innerHTML = "";
    const followedTokens = new Set(followed.map(String));
    for (const span of replica.querySelectorAll(".hb-token")) {
      const token = this.current.tokens[span.dataset.token];
      span.classList.toggle("hb-followed", followedTokens.has(String(token.index)));
    }

    const history = document.createElement("div");
    history.className = "hb-history";
    const status = document.createElement("div");
    status.className = "hb-history-status";
    status.textContent = statusText;
    const steps = document.createElement("div");
    steps.className = "hb-history-steps";
    history.append(status, steps);
    upper.replaceChildren(history);
    // Leave room for the popup above the line.
    const lineTop = this.current.lineElt.getBoundingClientRect().top;
    history.style.maxHeight = `${Math.max(150, lineTop - 120)}px`;
    this.position();
    return { history, status, steps };
  }

  /**
   * Show the peephole history of some of the line's tokens (see
   * `hyperblame::peephole`): how the window of tokens around them changed over
   * time, newest (nearest the line) to oldest.  Following several tokens (ex:
   * the whole line) follows their identifiers.  Older steps load as the user
   * scrolls up, until they've seen `MAX_AUTO_STEPS` steps or the server has
   * read `MAX_AUTO_COST` bytes, after which they need to ask for more.
   */
  static MAX_AUTO_STEPS = 50;
  static MAX_AUTO_COST = 1_000_000_000;

  showHistory(tokenIndices, what) {
    const { history, status, steps } = this.startFollowing(
      tokenIndices, `Loading the history of this ${what}…`);
    const legend = document.createElement("div");
    legend.className = "hb-history-legend";
    legend.innerHTML = `Each step shows the code after a commit, with the tokens it <span class="hb-step-changed">added</span> highlighted and the ${what == "line" ? "line's identifiers" : "token"} being followed <span class="hb-step-anchor">underlined</span>.`;
    status.after(legend);
    const state = {
      history,
      status,
      steps,
      next: `${this.info.peepholeUrl}/${tokenIndices.join(",")}.json`,
      loading: false,
      stepCount: 0,
      cost: 0,
      userScrolled: false,
    };
    history.addEventListener("scroll", () => {
      if (state.ignoreScroll) {
        state.ignoreScroll = false;
        return;
      }
      state.userScrolled = true;
      if (history.scrollTop < 40) {
        this.loadHistoryPage(state, false);
      }
    });
    this.loadHistoryPage(state, false);
  }

  async loadHistoryPage(state, userAsked) {
    if (state.loading || !state.next) {
      return;
    }
    if (!userAsked && (state.stepCount >= TokenBlamePopup.MAX_AUTO_STEPS ||
                       state.cost >= TokenBlamePopup.MAX_AUTO_COST)) {
      this.showKeepGoing(state);
      return;
    }
    state.loading = true;
    let page;
    try {
      page = await fetch(state.next).then(r => r.json());
    } catch (ex) {
      state.status.textContent = "Couldn't load the history.";
      state.loading = false;
      return;
    }
    state.loading = false;
    state.next = page.next;
    state.stepCount += page.steps.length;
    state.cost += page.cost;

    // Steps are newest first, and older steps go above newer ones, keeping
    // what the user is looking at in place.
    const { history, steps } = state;
    const fromBottom = history.scrollHeight - history.scrollTop;
    for (const step of page.steps) {
      steps.prepend(this.renderStep(step, page.commits[step.rev]));
    }
    state.ignoreScroll = true;
    history.scrollTop = history.scrollHeight - fromBottom;

    if (page.end) {
      state.status.textContent = {
        introduced: "The oldest commit above introduced all of the tokens followed.",
        root: "The oldest commit above has no parent.",
        lost: "Couldn't follow these tokens further back (they may have moved from another file).",
        missing: "The history doesn't go further back.",
      }[page.end] || "That's all.";
    } else if (!state.userScrolled && state.stepCount < 5) {
      // Fill a little without waiting for the user to scroll.
      state.status.textContent = "Scroll up for older changes.";
      this.loadHistoryPage(state, false);
    } else {
      state.status.textContent = "Scroll up for older changes.";
    }
    this.position();
  }

  /**
   * Show where the token is now, or the commit which removed it (see
   * `hyperblame::future`), with the changes to it along the way, oldest (at the
   * top) to newest.
   */
  async showFuture(token) {
    const { status, steps } = this.startFollowing(
      [token.index], "Following this token into the future…");
    let result;
    try {
      const url = this.info.peepholeUrl.replace(/\/peephole$/, "/future");
      result = await fetch(`${url}/${token.index}.json`).then(r => r.json());
    } catch (ex) {
      status.textContent = "Couldn't follow this token.";
      return;
    }
    const { future, commits } = result;
    const describe = {
      evolved: "changed it into another token",
      moved: "moved it to",
      renamed: "renamed its file to",
    };
    for (const change of future.changes) {
      const elt = document.createElement("div");
      elt.className = "hb-step";
      const header = document.createElement("div");
      header.className = "hb-step-header";
      header.innerHTML = commits[change.rev]?.header || change.rev.substring(0, 8);
      const what = document.createElement("div");
      what.append(`This commit ${describe[change.kind]}${change.kind == "evolved" ? "" : " " + change.path}: `);
      const view = document.createElement("a");
      view.textContent = "view";
      view.href = this.revLink(change.rev, change.path, `${change.token}`);
      what.append(view);
      elt.append(header, what);
      // Newer changes go below older ones, nearest the line.
      steps.append(elt);
    }

    status.textContent = "";
    if (future.outcome == "now") {
      status.append("This token is now ");
      const link = document.createElement("a");
      link.textContent = future.changes.length ? "here, after the changes below" : "here";
      link.href = `/${this.tree}/source/${this.encodePath(future.path)}#tokens=${future.token}`;
      status.append(link, ".");
    } else if (future.outcome == "removed" || future.outcome == "deleted") {
      status.append(future.outcome == "removed" ? "This token was removed by:" : "This token's file was deleted by:");
      const header = document.createElement("div");
      header.innerHTML = commits[future.rev]?.header || future.rev.substring(0, 8);
      const view = document.createElement("a");
      view.textContent = "view";
      view.href = `/${this.tree}/commit/${future.rev}`;
      header.append(" ", view);
      status.append(header);
    } else {
      status.textContent = "Couldn't follow this token further.";
    }
    this.position();
  }

  /**
   * Go to the latest version without the token: the parent of the commit which
   * introduced it, with the token it replaced (or the nearest token which
   * already existed) selected (see `hyperblame::peephole::before`), or just the
   * parent if we can't tell where it was.
   */
  async showBefore(token) {
    const url = this.info.peepholeUrl.replace(/\/peephole$/, "/before");
    try {
      const response = await fetch(`${url}/${token.index}.json`);
      if (response.ok) {
        const before = await response.json();
        document.location = this.revLink(before.rev, before.path, `${before.token}`);
        return;
      }
    } catch (ex) {
      // Fall back to the parent.
    }
    const commits = await this.data.getCommits();
    const info = commits[token.commit];
    if (info?.parent) {
      document.location = this.revLink(info.parent, this.info.paths[token.path]);
    }
  }

  showKeepGoing(state) {
    state.status.textContent = "";
    const button = document.createElement("button");
    button.textContent = "Keep going";
    button.addEventListener("click", () => {
      state.stepCount = 0;
      state.cost = 0;
      this.loadHistoryPage(state, true);
    });
    state.status.append(button);
  }

  /**
   * A step of a peephole history: the commit, and the window's lines as of
   * that commit, with the tokens it introduced highlighted.
   */
  renderStep(step, info) {
    const elt = document.createElement("div");
    elt.className = "hb-step";
    const header = document.createElement("div");
    header.className = "hb-step-header";
    header.innerHTML = info ? info.header : step.rev.substring(0, 8);
    const last = step.firstToken + step.tokens.length - 1;
    const view = document.createElement("a");
    view.className = "deemphasize";
    view.textContent = "view";
    view.href = this.revLink(step.stateRev, step.path, `${step.firstToken}-${last}`);
    header.append(" ", view);

    const code = document.createElement("pre");
    code.className = "hb-step-code";
    let pos = 0;
    step.tokens.forEach(([start, end, changed], i) => {
      code.append(step.text.slice(pos, start));
      const span = document.createElement("span");
      span.textContent = step.text.slice(start, end);
      if (changed) {
        span.classList.add("hb-step-changed");
      }
      if (step.anchors.includes(i)) {
        span.classList.add("hb-step-anchor");
      }
      code.append(span);
      pos = end;
    });
    code.append(step.text.slice(pos));
    elt.append(header, code);
    if (step.removed) {
      const removed = document.createElement("div");
      removed.className = "hb-step-removed";
      removed.textContent = `and removed ${step.removed} token${step.removed > 1 ? "s" : ""}`;
      elt.append(removed);
    }
    return elt;
  }

  renderRow(commit, info) {
    const [rev, time, author] = this.info.commits[commit];
    const row = document.createElement("div");
    row.className = "hb-row";
    row.dataset.commit = commit;
    row.style.setProperty("--hb-color", this.commitColor(commit));
    const summary = document.createElement("span");
    summary.className = "hb-row-summary";
    // The header is the summary line (with bug links) and then the author and
    // date; see `blame::commit_info_json`.
    summary.innerHTML = info ? info.header.split("\n<br>")[0] : rev.substring(0, 8);
    const meta = document.createElement("span");
    meta.className = "hb-row-meta";
    meta.textContent = `${this.info.authors[author]}, ${this.relativeDate(time)}`;
    row.append(summary, meta);
    return row;
  }

  /**
   * A copy of the line's `code` element with the text of each token wrapped in
   * `span.hb-token` elements (more than one if the token crosses syntax
   * highlighting boundaries).
   */
  renderReplica(code, tokens) {
    const replica = code.cloneNode(true);
    replica.removeAttribute("role");
    // On diffs, the line starts with its origin (ex: "+ "), before the
    // tokens' offsets.
    const shift = parseInt(code.dataset.hbOffset || "0", 10);
    tokens = tokens.map(t => ({ ...t, start: t.start + shift, end: t.end + shift }));
    // Each source line ends with a newline, which would add an empty line.
    const walker = document.createTreeWalker(replica, NodeFilter.SHOW_TEXT);
    let lastText = null;
    while (walker.nextNode()) {
      lastText = walker.currentNode;
    }
    if (lastText?.data.endsWith("\n")) {
      lastText.data = lastText.data.slice(0, -1);
    }
    // Clicking on tokens shows our menu rather than the symbol context menu.
    for (const elt of replica.querySelectorAll("[data-symbols]")) {
      elt.removeAttribute("data-symbols");
    }
    // Split the text nodes at token boundaries.
    const boundaries = new Set(tokens.flatMap(t => [t.start, t.end]));
    const textNodes = () => {
      const walker = document.createTreeWalker(replica, NodeFilter.SHOW_TEXT);
      const nodes = [];
      while (walker.nextNode()) {
        nodes.push(walker.currentNode);
      }
      return nodes;
    };
    let offset = 0;
    for (let node of textNodes()) {
      const end = offset + node.length;
      for (const boundary of [...boundaries].filter(b => b > offset && b < end).sort((a, b) => b - a)) {
        node.splitText(boundary - offset);
      }
      offset = end;
    }
    // Wrap the pieces inside tokens.
    offset = 0;
    let t = 0;
    for (const node of textNodes()) {
      const start = offset;
      offset += node.length;
      while (t < tokens.length && tokens[t].end <= start) {
        t++;
      }
      const token = tokens[t];
      if (token && token.start <= start && offset <= token.end) {
        const span = document.createElement("span");
        span.className = "hb-token";
        span.dataset.token = t;
        span.dataset.commit = token.commit;
        span.style.setProperty("--hb-color", this.commitColor(token.commit));
        node.replaceWith(span);
        span.append(node);
      }
    }
    return replica;
  }

  renderNote(elt, commits) {
    const note = document.createElement("div");
    note.className = "blame-entry";
    note.textContent = "This line has no tokens (it's blank).";
    // See `StripLine::strip_attrs` in `page_blame.rs`.
    const commitClass = [...elt.classList].find(c => c.startsWith("bc-"));
    if (elt.classList.contains("blame-interpolated") && commitClass) {
      const commit = parseInt(commitClass.substring(3), 10);
      const neighbors = {
        above: "the line above it, whose tokens were",
        below: "the line below it, whose tokens were",
      }[elt.dataset.interp] || "the lines around it, whose tokens were";
      note.append(` It's colored like ${neighbors} last changed in:`);
      const header = document.createElement("div");
      header.innerHTML = commits[commit]?.header || this.info.commits[commit][0];
      note.append(header);
    }
    return note;
  }

  renderDetails(commit, info, tokens) {
    const [rev] = this.info.commits[commit];
    const entry = document.createElement("div");
    entry.className = "blame-entry hb-entry";
    entry.dataset.commit = commit;
    entry.style.setProperty("--hb-color", this.commitColor(commit));
    const header = document.createElement("div");
    header.innerHTML = info ? info.header : rev;
    entry.append(header);

    // The commit's tokens on this line, by their path in the commit.
    const byPath = new Map();
    for (const token of tokens.filter(t => t.commit == commit)) {
      const path = this.info.paths[token.path];
      byPath.set(path, (byPath.get(path) || []).concat([token.lineno]));
    }
    const links = document.createElement("div");
    const addLink = (text, href, className) => {
      if (links.childNodes.length) {
        links.append(" · ");
      }
      const a = document.createElement("a");
      a.textContent = text;
      a.href = href;
      if (className) {
        a.className = className;
      }
      links.append(a);
    };
    const [path, linenos] = byPath.entries().next().value;
    addLink("diff", `/${this.tree}/diff/${rev}/${this.encodePath(path)}`);
    if (info?.phab) {
      addLink(`Phabricator ${info.phab.match(/\/(D[0-9]+)/)?.[1] || "revision"}`, info.phab);
    }
    if (info?.pr) {
      addLink(`Pull request #${info.pr.match(/\/([0-9]+)/)?.[1] || ""}`, info.pr);
    }
    if (info?.parent) {
      addLink("version before", this.revLink(info.parent, path), "deemphasize");
    }
    addLink("earliest version with these tokens", this.revLink(rev, path, this.tokenRanges(linenos)), "deemphasize");
    entry.append(links);
    return entry;
  }

  /**
   * "5-7,9" for [5, 6, 7, 9].
   */
  tokenRanges(indices) {
    const sorted = [...new Set(indices)].sort((a, b) => a - b);
    const ranges = [];
    for (const index of sorted) {
      const last = ranges[ranges.length - 1];
      if (last && last[1] + 1 == index) {
        last[1] = index;
      } else {
        ranges.push([index, index]);
      }
    }
    return ranges.map(([a, b]) => (a == b ? `${a}` : `${a}-${b}`)).join(",");
  }

  /**
   * The removals of tokens before, within, and after the line; see
   * `StripLine::strip_attrs` in `page_blame.rs`.
   */
  lineRemovals(elt, lineno) {
    const prevStrip = this.data.stripFor(lineno - 1);
    const removals = [];
    const add = (attr, where) => {
      for (const removal of BlamePopup.parseRemovals(attr, this.info)) {
        removals.push({ where, ...removal });
      }
    };
    add(elt.dataset.rmAbove, "at the start of the file");
    add(prevStrip?.dataset.rmBelow, "between the previous line and this line");
    add(elt.dataset.rmWithin, "within this line");
    add(elt.dataset.rmBelow, "between this line and the next line");
    return removals;
  }

  renderRemoval(removal, info) {
    const entry = document.createElement("div");
    entry.className = "blame-entry blame-removal";
    const count = removal.numRemoved == 1 ? "1 token was" : `${removal.numRemoved} tokens were`;
    let text = `${count} removed ${removal.where}`;
    if (removal.numMoved) {
      text += ` (${removal.numMoved} moved elsewhere)`;
    }
    entry.append(`${text} in:`);
    const header = document.createElement("div");
    header.innerHTML = info ? info.header : this.info.commits[removal.commit][0];
    entry.append(header);
    if (info?.parent) {
      const first = removal.firstToken;
      const last = first + removal.numRemoved - 1;
      const a = document.createElement("a");
      a.className = "deemphasize";
      a.textContent = "Show the removed tokens";
      a.href = this.revLink(info.parent, removal.path, last > first ? `${first}-${last}` : `${first}`);
      entry.append(a);
    }
    return entry;
  }

  commitColor(commit) {
    // `commit` may come from a data attribute.
    return this.colors.get(Number(commit)) || "currentColor";
  }

  /**
   * Draw a lane from each token's center up to a dot on its commit's row,
   * and move the rows' text to the right of the lanes.
   */
  drawLanes(root, svg, upper, replica) {
    const rootRect = root.getBoundingClientRect();
    const replicaTop = replica.getBoundingClientRect().top - rootRect.top;
    const centers = new Map();
    for (const span of root.querySelectorAll(".hb-replica .hb-token")) {
      const rect = span.getBoundingClientRect();
      const t = span.dataset.token;
      const [left, right] = centers.get(t) || [Infinity, -Infinity];
      centers.set(t, [Math.min(left, rect.left), Math.max(right, rect.right)]);
    }
    let maxX = 0;
    const lanes = [];
    for (const [t, [left, right]] of centers) {
      const x = (left + right) / 2 - rootRect.left;
      maxX = Math.max(maxX, x);
      const commit = root.querySelector(`.hb-replica .hb-token[data-token="${t}"]`).dataset.commit;
      lanes.push({ x, commit });
    }
    upper.style.setProperty("--hb-lanes-width", `${maxX + 12}px`);

    svg.setAttribute("height", replicaTop);
    const ns = "http://www.w3.org/2000/svg";
    const rowYs = new Map();
    for (const row of upper.querySelectorAll(".hb-row")) {
      const rect = row.getBoundingClientRect();
      rowYs.set(row.dataset.commit, rect.top + rect.height / 2 - rootRect.top);
    }
    const rowLefts = new Map();
    for (const { x, commit } of lanes) {
      rowLefts.set(commit, Math.min(rowLefts.get(commit) ?? Infinity, x));
    }
    for (const [commit, y] of rowYs) {
      const track = document.createElementNS(ns, "line");
      track.setAttribute("x1", rowLefts.get(commit));
      track.setAttribute("x2", maxX + 8);
      track.setAttribute("y1", y);
      track.setAttribute("y2", y);
      track.setAttribute("class", "hb-track");
      track.dataset.commit = commit;
      track.style.setProperty("--hb-color", this.commitColor(commit));
      svg.append(track);
    }
    for (const { x, commit } of lanes) {
      const y = rowYs.get(commit);
      const line = document.createElementNS(ns, "line");
      line.setAttribute("x1", x);
      line.setAttribute("x2", x);
      line.setAttribute("y1", y);
      line.setAttribute("y2", replicaTop);
      line.setAttribute("class", "hb-lane");
      line.dataset.commit = commit;
      line.style.setProperty("--hb-color", this.commitColor(commit));
      const dot = document.createElementNS(ns, "circle");
      dot.setAttribute("cx", x);
      dot.setAttribute("cy", y);
      dot.setAttribute("r", 3);
      dot.setAttribute("class", "hb-dot");
      dot.dataset.commit = commit;
      dot.style.setProperty("--hb-color", this.commitColor(commit));
      svg.append(line, dot);
    }
  }

  bindInteractions(root, tokens) {
    const setHot = commit => {
      root.classList.toggle("hb-has-hot", commit !== null);
      for (const elt of root.querySelectorAll("[data-commit]")) {
        elt.classList.toggle("hb-hot", elt.dataset.commit === commit);
      }
    };
    root.addEventListener("mouseover", event => {
      // The commits' rows are gone when following tokens.
      if (root.classList.contains("hb-history-mode")) {
        return;
      }
      const target = event.target.closest?.("[data-commit]");
      setHot(target ? target.dataset.commit : null);
    });
    root.addEventListener("mouseleave", () => setHot(null));
    root.addEventListener("click", event => {
      const span = event.target.closest?.(".hb-token");
      if (span) {
        this.showTokenMenu(event, tokens[span.dataset.token]);
      }
    });
  }

  /**
   * Show the context menu for a token in the replica.
   */
  showTokenMenu(event, token) {
    const [rev] = this.info.commits[token.commit];
    const items = [
      new MenuItem({
        html: "Show the earliest version with this token",
        href: this.revLink(rev, this.info.paths[token.path], `${token.lineno}`),
        icon: "export-alt",
        section: "hyperblame",
      }),
    ];
    if (this.info.peepholeUrl) {
      items.push(new MenuItem({
        html: "Show the latest version without this token",
        action: () => {
          ContextMenu.hide();
          this.showBefore(token);
        },
        icon: "export-alt",
        section: "hyperblame",
      }));
    }
    if (token.pred) {
      items.push(new MenuItem({
        html: "Show the earliest version of the token it replaced",
        href: this.revLink(this.info.commits[token.pred.commit][0],
                           this.info.paths[token.pred.path], `${token.pred.lineno}`),
        icon: "export-alt",
        section: "hyperblame",
      }));
    }
    if (this.info.peepholeUrl) {
      items.push(new MenuItem({
        html: "Follow this token into the past",
        action: () => {
          ContextMenu.hide();
          this.showHistory([token.index], "token");
        },
        icon: "export-alt",
        section: "hyperblame",
      }));
      items.push(new MenuItem({
        html: "Follow this line into the past",
        action: () => {
          ContextMenu.hide();
          this.showHistory(this.current.tokens.map(t => t.index), "line");
        },
        icon: "export-alt",
        section: "hyperblame",
      }));
      // Pages for the tip have nothing to follow into.
      if (this.info.dataUrl.includes("/rev-hyperblame/")) {
        items.push(new MenuItem({
          html: "Follow this token into the future",
          action: () => {
            ContextMenu.hide();
            this.showFuture(token);
          },
          icon: "export-alt",
          section: "hyperblame",
        }));
      }
    }
    // (The page's `#tokens=` hash is for the file in the page's revision.)
    if (this.data === HyperblameContexts.contexts[0]) {
      items.push(new MenuItem({
        html: "Select this token's line",
        href: `#tokens=${token.index}`,
        preaction: () => ContextMenu.hide(),
        icon: "export-alt",
        section: "hyperblame",
      }));
    }
    ContextMenu.showItems(items, event);
  }
})();
