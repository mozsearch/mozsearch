/**
 * The token-centric blame popup (see "Token blame UI plan" in the hyperblame
 * notes).  Pages with token-centric blame have `BLAME_INFO` (see
 * `page_blame.rs`), which names the page's "hyperblame" data files: what the
 * popup shows about each commit (`commits.json`), and the tokens of the lines
 * in chunks (`lines-K.json`, see `LinesChunk`).
 */

/**
 * Loads the page's hyperblame data files, starting with the commits and the
 * chunk with the page's first selected line (or its first line), and loading
 * other chunks as they near the viewport or when they're needed.
 */
var HyperblameData = new (class HyperblameData {
  constructor() {
    this.available = typeof BLAME_INFO !== "undefined" && !!BLAME_INFO.dataUrl;
    if (!this.available) {
      return;
    }
    this.commitsPromise = null;
    // Chunk promises by chunk index.
    this.chunkPromises = new Map();

    // The (1-based) index of each line's first token.
    this.lineFirstTokens = [];
    let total = 0;
    for (const count of BLAME_INFO.tokenCounts) {
      this.lineFirstTokens.push(total + 1);
      total += count;
    }

    this.getCommits();
    const selected = document.querySelector(".source-line-with-number.highlighted");
    const firstLine = selected ? parseInt(selected.id.substring("line-".length), 10) : 1;
    this.getChunk(this.chunkForLine(firstLine));

    // Load chunks when their first line gets near the viewport.
    if (BLAME_INFO.chunks.length > 1) {
      const observer = new IntersectionObserver(entries => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            observer.unobserve(entry.target);
            this.getChunk(parseInt(entry.target.dataset.hyperblameChunk, 10));
          }
        }
      }, { rootMargin: "100% 0px" });
      BLAME_INFO.chunks.forEach((firstLine, k) => {
        const line = document.getElementById(`line-${firstLine + 1}`);
        if (line) {
          line.dataset.hyperblameChunk = k;
          observer.observe(line);
        }
      });
    }
  }

  getCommits() {
    if (!this.commitsPromise) {
      this.commitsPromise = fetch(`${BLAME_INFO.dataUrl}/commits.json`).then(r => r.json());
    }
    return this.commitsPromise;
  }

  /**
   * The index of the chunk with the given (1-based) line.
   */
  chunkForLine(lineno) {
    const chunks = BLAME_INFO.chunks;
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
        fetch(`${BLAME_INFO.dataUrl}/lines-${k}.json`)
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
    if (!HyperblameData.available) {
      return false;
    }
    const lineElt = elt.closest(".source-line-with-number");
    const lineno = parseInt(lineElt.id.substring("line-".length), 10);
    let commits, tokens;
    try {
      [commits, tokens] = await Promise.all([
        HyperblameData.getCommits(),
        HyperblameData.getLineTokens(lineno),
      ]);
    } catch (ex) {
      // The popup without the data will do.
      console.error("Couldn't load the hyperblame data:", ex);
      return false;
    }
    if (BlamePopup.triggerElement != elt) {
      return true;
    }

    // The line's commits, newest first, which get distinct colors.
    const lineCommits = [...new Set(tokens.map(t => t.commit))];
    lineCommits.sort((a, b) => BLAME_INFO.commits[b][1] - BLAME_INFO.commits[a][1]);
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
      next: `${BLAME_INFO.peepholeUrl}/${tokenIndices.join(",")}.json`,
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
      const url = BLAME_INFO.peepholeUrl.replace(/\/peephole$/, "/future");
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
    const [rev, time, author] = BLAME_INFO.commits[commit];
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
    meta.textContent = `${BLAME_INFO.authors[author]}, ${this.relativeDate(time)}`;
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
      header.innerHTML = commits[commit]?.header || BLAME_INFO.commits[commit][0];
      note.append(header);
    }
    return note;
  }

  renderDetails(commit, info, tokens) {
    const [rev] = BLAME_INFO.commits[commit];
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
      const path = BLAME_INFO.paths[token.path];
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
    const prevStrip = document.querySelector(`#line-${lineno - 1} .blame-strip`);
    const removals = [];
    const add = (attr, where) => {
      for (const removal of BlamePopup.parseRemovals(attr)) {
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
    header.innerHTML = info ? info.header : BLAME_INFO.commits[removal.commit][0];
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
    const [rev] = BLAME_INFO.commits[token.commit];
    const items = [
      new MenuItem({
        html: "Show the earliest version with this token",
        href: this.revLink(rev, BLAME_INFO.paths[token.path], `${token.lineno}`),
        icon: "export-alt",
        section: "hyperblame",
      }),
    ];
    if (token.pred) {
      items.push(new MenuItem({
        html: "Show the earliest version of the token it replaced",
        href: this.revLink(BLAME_INFO.commits[token.pred.commit][0],
                           BLAME_INFO.paths[token.pred.path], `${token.pred.lineno}`),
        icon: "export-alt",
        section: "hyperblame",
      }));
    }
    if (BLAME_INFO.peepholeUrl) {
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
      if (BLAME_INFO.dataUrl.includes("/rev-hyperblame/")) {
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
    items.push(new MenuItem({
      html: "Select this token's line",
      href: `#tokens=${token.index}`,
      preaction: () => ContextMenu.hide(),
      icon: "export-alt",
      section: "hyperblame",
    }));
    ContextMenu.showItems(items, event);
  }
})();
