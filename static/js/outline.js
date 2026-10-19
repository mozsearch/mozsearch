/**
 * The fancy bar's "Outline" section, on pages of files of trees with
 * token-centric history: the file's definitions, nested, each with a badge
 * saying how it changed since a baseline and a sparkline of the commits which
 * changed it since then, from the outline diff of the page's revision from
 * the baseline (see `hyperblame::outline_diff`, which the web-server serves
 * as `/{tree}/rev-outline/{from}/{to}/{path}`).  The baseline is the latest
 * Firefox version cut before the revision on trees with them (see
 * file-timeline.js's `allCuts`), or else 4 weeks before it, unless the file
 * timeline's "since" picks another (a commit or a cut; see file-timeline.js's
 * `setBaseline`).  It lists the definitions
 * which changed (with the ones they're in), or all of them with "All".
 */
var OutlineSection = new (class OutlineSection {
  constructor() {
    const content = document.getElementById("panel-content");
    const info = typeof BLAME_INFO !== "undefined" ? BLAME_INFO : null;
    this.tree = document.getElementById("data")?.dataset.tree;
    this.path = FileTimeline.path();
    this.rev = FileTimeline.pageRev();
    if (!content || !Settings.fancyBar.enabled || !info?.dataUrl || !this.tree || !this.path || !this.rev) {
      return;
    }

    const h4 = document.createElement("h4");
    h4.textContent = "Outline";
    const section = document.createElement("div");
    section.id = "panel-outline";
    section.className = "outline-section";
    const header = document.createElement("div");
    header.className = "outline-header";
    this.baselineNode = document.createElement("span");
    this.baselineNode.className = "outline-baseline";
    const allLabel = document.createElement("label");
    allLabel.className = "outline-all";
    allLabel.title = "List all of the file's definitions, not only the ones which changed";
    this.allCheckbox = document.createElement("input");
    this.allCheckbox.type = "checkbox";
    this.allCheckbox.addEventListener("change", () => this.filter());
    allLabel.append(this.allCheckbox, "All");
    header.append(this.baselineNode, allLabel);
    this.summary = document.createElement("div");
    this.summary.className = "outline-summary";
    this.list = document.createElement("ul");
    this.list.className = "outline-list";
    section.append(header, this.summary, this.list);

    // (Before the debug section, if any.)
    const debug = [...content.querySelectorAll("h4")].find(node => node.textContent == "Debug");
    if (debug) {
      debug.before(h4, section);
    } else {
      content.append(h4, section);
    }

    document.addEventListener("baselinechange", event => this.load(event.detail));
    this.load(null);
  }

  /** The default baseline: see the class's comment. */
  async defaultBaseline() {
    const revTime = FileTimeline.revTime ?? Date.now() / 1000;
    const latest = (await FileTimeline.allCuts()).filter(cut => cut.time <= revTime).pop();
    if (latest) {
      return {
        time: latest.time,
        from: latest.parent,
        label: `Firefox ${latest.version}`,
        description: `Firefox ${latest.version}'s cut from mozilla-central, the latest before this revision`,
      };
    }
    const time = revTime - 28 * 24 * 60 * 60;
    return {
      time,
      from: `since:${FileTimeline.day(time)}`,
      label: FileTimeline.day(time),
      description: "4 weeks before this revision",
    };
  }

  /** Compare the outline with `baseline` (see `FileTimeline.setBaseline`), or the default one. */
  async load(baseline) {
    const loading = Symbol();
    this.loading = loading;
    baseline ??= await this.defaultBaseline();
    if (this.loading !== loading) {
      return;
    }
    this.baseline = baseline;
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = baseline.label;
    button.title = `${baseline.description ? `${baseline.description}; ` : ""}pick another baseline on the file's timeline`;
    button.addEventListener("click", () => {
      if (FileTimeline.button) {
        FileTimeline.togglePopup(FileTimeline.button);
      }
    });
    this.baselineNode.replaceChildren("Changes since ", button);
    this.summary.textContent = "Loading\u2026";
    this.list.replaceChildren();
    let diff;
    try {
      const response = await fetch(`/${this.tree}/rev-outline/${encodeURIComponent(baseline.from)}/${this.rev}/${this.path}`);
      if (!response.ok) {
        throw new Error(await response.text());
      }
      diff = await response.json();
    } catch (e) {
      if (this.loading === loading) {
        this.summary.textContent = `The outline couldn't be compared: ${e.message}`;
      }
      return;
    }
    if (this.loading === loading) {
      this.render(diff);
    }
  }

  render(diff) {
    this.diff = diff;
    const counts = new Map();
    for (const entry of diff.entries) {
      counts.set(entry.status, (counts.get(entry.status) ?? 0) + 1);
    }
    const parts = [["changed", "changed"], ["added", "added"], ["removed", "removed"],
                   ["renamed", "renamed"], ["moved", "moved in"]]
      .filter(([status]) => counts.get(status))
      .map(([status, text]) => `${counts.get(status)} ${text}`);
    const commits = diff.commits.reduce((sum, commit) => sum + (commit.count ?? 1), 0);
    this.summary.textContent = parts.length
      ? `${parts.join(", ")}, in ${commits} commit${commits == 1 ? "" : "s"}`
      : `No definitions changed${commits ? `, in ${commits} commit${commits == 1 ? "" : "s"}` : ""}`;
    if (!diff.complete) {
      this.summary.append(" (the history doesn't reach back that far)");
    }
    const compare = document.createElement("a");
    compare.className = "outline-compare";
    compare.href = FileTimeline.outlineDiffURL(this.baseline.from, this.rev);
    compare.textContent = "compare";
    compare.title = "The file's outline then and now, side by side";
    this.summary.append(" (", compare, ")");

    // (The sparklines' scale, for comparing them.)
    const fromTime = Date.parse(diff.from.date);
    const span = Math.max(1, Date.parse(diff.to.date) - fromTime);
    const most = Math.max(1, ...diff.entries.flatMap(entry => (entry.changes ?? []).map(([, a, r]) => a + r)));

    const items = diff.entries.map((entry, index) => {
      const item = document.createElement("li");
      item.className = `outline-entry outline-${entry.status}`;
      item.dataset.index = index;
      item.style.setProperty("--outline-depth", entry.depth);
      // (Removed ones are in the baseline's file.)
      const removed = entry.status == "removed";
      const lines = removed ? entry.from_lines : entry.lines;
      const name = document.createElement(lines ? "a" : "span");
      name.className = "outline-name";
      name.textContent = entry.name;
      name.title = `${entry.pretty} (${entry.decl ? "declaration" : entry.kind})` +
        (lines ? `, lines ${lines[0]}\u2013${lines[1]}` : "") +
        (removed ? `, in ${diff.from.rev.slice(0, 12)}` : "");
      if (lines) {
        name.href = removed
          ? `/${this.tree}/rev/${diff.from.rev}/${diff.from_path ?? this.path}#${lines[0]}`
          : `#${lines[0]}`;
      }
      item.append(name);
      const badge = this.badge(entry);
      if (badge) {
        item.append(" ", badge);
      }
      if (entry.changes?.length) {
        item.append(" ", this.sparkline(entry, diff, fromTime, span, most));
      }
      return item;
    });
    this.items = items;
    this.list.replaceChildren(...items);
    this.filter();
  }

  badge(entry) {
    const tokens = entry.tokens ?? { added: 0, removed: 0 };
    const badge = document.createElement("span");
    badge.className = "outline-badge";
    const delta = [tokens.added ? `+${tokens.added}` : "", tokens.removed ? `\u2212${tokens.removed}` : ""]
      .filter(Boolean).join(" ");
    const changes = `${tokens.added} tokens added and ${tokens.removed} removed in ` +
      `${entry.changes?.length ?? 0} commit${entry.changes?.length == 1 ? "" : "s"}`;
    switch (entry.status) {
      case "changed":
        badge.textContent = delta;
        badge.title = changes;
        break;
      case "added":
        badge.textContent = "new";
        badge.title = `Added since ${this.baseline.label}`;
        break;
      case "removed":
        badge.textContent = entry.evolved_into ? "moved" : "removed";
        badge.title = entry.evolved_into
          ? `Moved to ${entry.evolved_into.pretty} in ${entry.evolved_into.path}`
          : `Removed since ${this.baseline.label}`;
        break;
      case "renamed":
        badge.textContent = delta ? `renamed ${delta}` : "renamed";
        badge.title = `Renamed from ${entry.evolved_from.pretty}; ${changes}`;
        break;
      case "moved":
        badge.textContent = "moved in";
        badge.title = `Moved from ${entry.evolved_from.pretty} in ${entry.evolved_from.path}`;
        break;
      default:
        return null;
    }
    return badge;
  }

  /** A bar for each commit which changed `entry`, by its time and size. */
  sparkline(entry, diff, fromTime, span, most) {
    const NS = "http://www.w3.org/2000/svg";
    const [width, height] = [40, 12];
    const svg = document.createElementNS(NS, "svg");
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.classList.add("outline-spark");
    for (const [index, added, removed] of entry.changes) {
      const commit = diff.commits[index];
      const x = Math.min(width - 2, Math.max(0, ((Date.parse(commit.date) - fromTime) / span) * width));
      const h = Math.max(2, (height * Math.log1p(added + removed)) / Math.log1p(most));
      const bar = document.createElementNS(NS, "rect");
      bar.setAttribute("x", x);
      bar.setAttribute("y", height - h);
      bar.setAttribute("width", 2);
      bar.setAttribute("height", h);
      const title = document.createElementNS(NS, "title");
      title.textContent = `${commit.date.slice(0, 10)} ${commit.subject}` +
        (commit.count > 1 ? ` (and the week's other ${commit.count - 1})` : "") +
        `: +${added} \u2212${removed}`;
      bar.append(title);
      svg.append(bar);
    }
    return svg;
  }

  /** Show the entries which changed and the ones they're in, or all of them. */
  filter() {
    if (!this.items) {
      return;
    }
    const all = this.allCheckbox.checked;
    const shown = new Set();
    // (The entries the current one is in, by depth.)
    const ancestors = [];
    this.diff.entries.forEach((entry, index) => {
      ancestors.length = entry.depth;
      if (all || entry.status != "unchanged") {
        shown.add(index);
        for (const ancestor of ancestors) {
          shown.add(ancestor);
        }
      }
      ancestors[entry.depth] = index;
    });
    this.items.forEach((item, index) => {
      item.hidden = !shown.has(index);
    });
  }
})();
