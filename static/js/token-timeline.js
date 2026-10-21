/**
 * The token-timeline: view (see tools/src/hyperblame/token_timeline.rs): a
 * line a token (with its case variants), of how many more of its instances
 * than now the tree had each week (the journals count instances added and
 * removed, not instances), with markers at the weeks changing the lines the
 * most.  Hovering (or focusing) a marker shows what its week's commits
 * changed, and the other tokens they changed the most, with links to their
 * timelines (alone, added to this one, or all of them); clicking it keeps it
 * shown.  Hovering the chart shows the lines' values at that week.  The lines
 * share a scale, or each fills the chart ("Scale each line to its range"),
 * for tokens of different magnitudes.
 */
var TokenTimeline = new (class TokenTimeline {
  constructor() {
    this.model = typeof TOKEN_TIMELINE !== "undefined" ? TOKEN_TIMELINE : null;
    this.root = document.getElementById("token-timeline");
    if (!this.model || !this.root) {
      return;
    }
    this.tree = document.location.pathname.split("/")[1];
    this.timeline = this.model.timeline;
    // (Tableau's 10 and Dark2's 8: enough for MAX_TOKENS.)
    this.colors = [...d3.schemeTableau10, ...d3.schemeDark2];
    this.ownRange = false;
    this.pinned = null;
    this.render();
  }

  /** The query URL of the timelines of `tokens`. */
  queryUrl(tokens) {
    const query = tokens.map(token => `token-timeline:${token}`).join(" ");
    return `/${this.tree}/query/default?q=${encodeURIComponent(query)}`;
  }

  /** The tokens of the lines, as asked. */
  tokens() {
    return this.timeline.series.map(series => series.token);
  }

  /** An element `tag` with `className` and `text`. */
  el(tag, className, text) {
    const el = document.createElement(tag);
    if (className) {
      el.className = className;
    }
    if (text != null) {
      el.textContent = text;
    }
    return el;
  }

  link(href, text, className) {
    const a = this.el("a", className, text);
    a.href = href;
    return a;
  }

  /** "+3 −5" for a change. */
  change({ added, removed }) {
    const parts = [];
    if (added) {
      parts.push(`+${added.toLocaleString()}`);
    }
    if (removed) {
      parts.push(`−${removed.toLocaleString()}`);
    }
    return parts.join(" ") || "0";
  }

  render() {
    this.root.replaceChildren();
    if (this.model.unsupported) {
      this.root.append(this.el("p", "token-timeline-empty", "This tree has no token-centric history."));
      return;
    }
    const timeline = this.timeline;
    if (timeline.dropped.length) {
      this.root.append(this.el("p", "token-timeline-dropped",
        `A timeline has ${timeline.series.length} tokens at most; left out: ${timeline.dropped.join(", ")}.`));
    }
    this.renderLegend();
    const chart = this.el("div", "token-timeline-chart");
    this.root.append(chart);
    this.chart = chart;
    this.renderChart();
    this.popup = this.el("div", "token-timeline-popup");
    this.popup.hidden = true;
    this.root.append(this.popup);
    document.addEventListener("keydown", event => {
      if (event.key == "Escape") {
        this.hidePopup(true);
      }
    });
    document.addEventListener("click", event => {
      if (this.pinned && !this.popup.contains(event.target) && !event.target.closest(".token-timeline-marker")) {
        this.hidePopup(true);
      }
    });
  }

  renderLegend() {
    const legend = this.el("ul", "token-timeline-legend");
    this.timeline.series.forEach((series, index) => {
      const item = this.el("li");
      const swatch = this.el("span", "token-timeline-swatch");
      swatch.style.backgroundColor = this.colors[index % this.colors.length];
      item.append(swatch, this.el("code", null, series.token));
      const others = series.variants.map(v => v.token).filter(token => token != series.token);
      const notes = [];
      if (!series.weeks.length) {
        notes.push("no history");
      } else {
        const added = series.variants.reduce((sum, v) => sum + v.added, 0);
        const removed = series.variants.reduce((sum, v) => sum + v.removed, 0);
        notes.push(this.change({ added, removed }));
        if (!series.read_all) {
          notes.push(`since ${series.weeks[0].date.slice(0, 4)}`);
        }
      }
      if (others.length) {
        notes.push(`with ${others.join(", ")}`);
      }
      item.append(" ", this.el("span", "token-timeline-legend-note", `(${notes.join("; ")})`), " ");
      item.append(this.link(`/${this.tree}/query/default?q=${encodeURIComponent(`token-history:${series.token}`)}`,
                            "history", "token-timeline-history-link"));
      legend.append(item);
    });
    const scale = this.el("label", "token-timeline-scale");
    const checkbox = this.el("input");
    checkbox.type = "checkbox";
    checkbox.addEventListener("change", () => {
      this.ownRange = checkbox.checked;
      this.renderChart();
    });
    scale.append(checkbox, " Scale each line to its range");
    this.root.append(legend, scale);
  }

  /** A line's points: its value before its first week, each week's end, now. */
  points(series, start) {
    const points = [{ date: start, value: series.start }];
    for (const week of series.weeks) {
      points.push({ date: new Date(week.date), value: week.relative, week });
    }
    points.push({ date: new Date(), value: 0 });
    return points;
  }

  renderChart() {
    const chart = this.chart;
    chart.replaceChildren();
    const series = this.timeline.series.filter(s => s.weeks.length);
    if (!series.length) {
      chart.append(this.el("p", "token-timeline-empty", "The history has nothing for these tokens."));
      return;
    }
    const width = Math.max(600, this.root.clientWidth || 900);
    const height = 360;
    const margin = { top: 24, right: 16, bottom: 28, left: 64 };
    const firstDates = series.map(s => new Date(s.weeks[0].date));
    // (A week before the first week, so its step shows.)
    const start = new Date(Math.min(...firstDates) - 7 * 24 * 3600 * 1000);
    const x = d3.scaleUtc().domain([start, new Date()]).range([margin.left, width - margin.right]);

    // Each line's values, or its values within its range (0 to 1).
    const lines = this.timeline.series.map((s, index) => {
      if (!s.weeks.length) {
        return null;
      }
      const points = this.points(s, start);
      const values = points.map(p => p.value);
      const [min, max] = [Math.min(...values), Math.max(...values)];
      const scaled = this.ownRange
        ? points.map(p => ({ ...p, y: (p.value - min) / Math.max(1, max - min) }))
        : points.map(p => ({ ...p, y: p.value }));
      return { series: s, index, points: scaled };
    });
    const all = lines.filter(Boolean).flatMap(line => line.points.map(p => p.y));
    const y = d3.scaleLinear()
      .domain(this.ownRange ? [0, 1] : [Math.min(0, ...all), Math.max(0, ...all)])
      .nice()
      .range([height - margin.bottom, margin.top]);

    const svg = d3.select(chart).append("svg")
      .attr("class", "token-timeline-svg")
      .attr("width", width)
      .attr("height", height)
      .attr("role", "img")
      .attr("aria-label", `Timelines of ${this.tokens().join(", ")}`);
    svg.append("g")
      .attr("transform", `translate(0,${height - margin.bottom})`)
      .call(d3.axisBottom(x).ticks(width / 90));
    svg.append("g")
      .attr("transform", `translate(${margin.left},0)`)
      .call(this.ownRange ? d3.axisLeft(y).ticks(5, "%") : d3.axisLeft(y).ticks(6));
    svg.append("text")
      .attr("class", "token-timeline-axis-label")
      .attr("x", margin.left)
      .attr("y", 12)
      .text(this.ownRange ? "Instances, within each line's range" : "Instances more than now");
    if (!this.ownRange) {
      svg.append("line")
        .attr("class", "token-timeline-now")
        .attr("x1", margin.left).attr("x2", width - margin.right)
        .attr("y1", y(0)).attr("y2", y(0));
    }

    // The markers, under the lines.
    const markers = svg.append("g").attr("class", "token-timeline-markers");
    for (const marker of this.timeline.markers) {
      const mx = x(new Date(marker.date));
      const g = markers.append("g")
        .attr("class", "token-timeline-marker")
        .attr("tabindex", 0)
        .attr("role", "button")
        .attr("aria-label", `The week ${marker.week}'s changes`)
        .attr("data-week", marker.week);
      g.append("line")
        .attr("x1", mx).attr("x2", mx)
        .attr("y1", margin.top).attr("y2", height - margin.bottom);
      g.append("path")
        .attr("d", `M${mx - 5},${margin.top - 8} L${mx + 5},${margin.top - 8} L${mx},${margin.top} Z`);
      // (A wider target than the line.)
      g.append("rect")
        .attr("class", "token-timeline-marker-target")
        .attr("x", mx - 5).attr("y", margin.top - 8)
        .attr("width", 10).attr("height", height - margin.bottom - margin.top + 8);
      const node = g.node();
      const show = () => this.showPopup(marker, node, mx);
      node.addEventListener("mouseenter", () => {
        if (!this.pinned) {
          show();
        }
      });
      node.addEventListener("focus", show);
      node.addEventListener("click", event => {
        event.stopPropagation();
        show();
        this.pinned = marker.week;
      });
      node.addEventListener("keydown", event => {
        if (event.key == "Enter" || event.key == " ") {
          event.preventDefault();
          show();
          this.pinned = marker.week;
        }
      });
    }

    const line = d3.line().curve(d3.curveStepAfter).x(p => x(p.date)).y(p => y(p.y));
    for (const l of lines.filter(Boolean)) {
      svg.append("path")
        .attr("class", "token-timeline-line")
        .attr("data-token", l.series.token)
        .attr("stroke", this.colors[l.index % this.colors.length])
        .attr("d", line(l.points));
    }

    // The values at the hovered week.
    const cursor = svg.append("line").attr("class", "token-timeline-cursor")
      .attr("y1", margin.top).attr("y2", height - margin.bottom).attr("visibility", "hidden");
    const readout = this.el("div", "token-timeline-readout");
    readout.hidden = true;
    chart.append(readout);
    svg.on("mousemove", event => {
      const [px] = d3.pointer(event);
      if (px < margin.left || px > width - margin.right) {
        return;
      }
      const date = x.invert(px);
      cursor.attr("x1", px).attr("x2", px).attr("visibility", "visible");
      readout.replaceChildren(this.el("div", "token-timeline-readout-date", date.toISOString().slice(0, 10)));
      for (const l of lines.filter(Boolean)) {
        const at = l.points.filter(p => p.date <= date).pop() ?? l.points[0];
        const row = this.el("div");
        const swatch = this.el("span", "token-timeline-swatch");
        swatch.style.backgroundColor = this.colors[l.index % this.colors.length];
        row.append(swatch, this.el("code", null, l.series.token), ` ${at.value.toLocaleString()}`);
        readout.append(row);
      }
      readout.hidden = false;
      readout.style.left = `${Math.min(px + 12, width - 180)}px`;
    });
    svg.on("mouseleave", () => {
      cursor.attr("visibility", "hidden");
      readout.hidden = true;
    });
  }

  hidePopup(unpin) {
    if (unpin) {
      this.pinned = null;
    }
    if (!this.pinned) {
      this.popup.hidden = true;
    }
  }

  /** The popup of `marker`'s week, by its line at `mx`. */
  showPopup(marker, node, mx) {
    const popup = this.popup;
    popup.replaceChildren();
    popup.dataset.week = marker.week;
    popup.append(this.el("div", "token-timeline-popup-week", `The week ${marker.week} (from ${marker.date})`));
    const changes = this.el("div", "token-timeline-popup-changes");
    for (const change of marker.changes) {
      const span = this.el("span", "token-timeline-popup-change");
      const swatch = this.el("span", "token-timeline-swatch");
      swatch.style.backgroundColor = this.colors[change.series % this.colors.length];
      span.append(swatch, this.el("code", null, change.token), ` ${this.change(change)}`);
      changes.append(span, " ");
    }
    popup.append(changes);

    const tokens = this.tokens();
    const commits = this.el("ul", "token-timeline-commits");
    for (const commit of marker.commits) {
      const item = this.el("li", "token-timeline-commit");
      item.append(this.link(`/${this.tree}/commit/${commit.rev}`, commit.rev.slice(0, 12), "token-timeline-rev"),
                  ` ${commit.date}`);
      if (commit.header) {
        const header = this.el("span", "token-timeline-header");
        // (The header's HTML, escaped and with its bugs linked by the server.)
        header.innerHTML = commit.header;
        item.append(": ", header);
      }
      const changed = this.el("div", "token-timeline-commit-changes");
      changed.textContent = commit.changes.map(c => `${c.token} ${this.change(c)}`).join("; ");
      item.append(changed);
      if (commit.others.length) {
        const others = this.el("div", "token-timeline-others");
        others.append("Also changed: ");
        commit.others.forEach((other, i) => {
          const span = this.el("span", "token-timeline-other");
          span.append(this.link(this.queryUrl([other.token]), other.token, "token-timeline-other-link"));
          span.append(` (${this.change(other)})`);
          if (tokens.length < 16 && !tokens.some(t => t.toLowerCase() == other.token.toLowerCase())) {
            span.append(" ", this.link(this.queryUrl([...tokens, other.token]), "+", "token-timeline-add"));
            span.lastChild.title = `Add ${other.token} to this timeline`;
          }
          others.append(span);
          if (i < commit.others.length - 1) {
            others.append(", ");
          }
        });
        others.append(" ", this.link(this.queryUrl(commit.others.map(o => o.token)),
                                     `all ${commit.others.length} timelines`, "token-timeline-all"));
        item.append(others);
      } else if (!commit.others_known && commit.header) {
        item.append(this.el("div", "token-timeline-others",
                            "(Too many files changed for their tokens to be listed.)"));
      }
      commits.append(item);
    }
    popup.append(commits);
    if (marker.week_commits > marker.commits.length) {
      popup.append(this.el("div", "token-timeline-more",
        `and ${marker.week_commits - marker.commits.length} other commits that week`));
    }
    // (Below the chart, by the marker, so that it doesn't hide the lines.)
    popup.hidden = false;
    const width = this.root.clientWidth || 900;
    popup.style.left = `${Math.max(0, Math.min(mx - 40, width - popup.offsetWidth - 8))}px`;
    popup.style.top = `${this.chart.offsetTop + this.chart.offsetHeight + 4}px`;
  }
})();
