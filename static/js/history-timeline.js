/**
 * A history grep's timeline (`text-timeline:`, `re-timeline:`; see `Timeline`
 * in tools/src/hyperblame/history_grep.rs): a step line per series (the
 * matches' count over the files, each file's captured number, or how many
 * distinct values the matches captured), with a marker at each revision
 * changing it, whose tooltip says what it added and removed, and which links
 * to the revision.  Where there are too many revisions for a marker each (ex:
 * thousands over 25 years), a marker groups those of a day, week, month,
 * quarter or year (by when they landed; see `grouping`), whose tooltip lists
 * them, and which zooms the chart into its time.  The table below the chart
 * (history_grep_root.liquid) has the same changes.
 */

/** How close (in pixels) markers may be. */
const MIN_MARKER_SPACING = 8;
const DAY_MS = 24 * 3600 * 1000;
/** How many revisions a group's tooltip lists. */
const MAX_GROUP_TOOLTIP_REVISIONS = 10;

var HistoryTimeline = new (class HistoryTimeline {
  constructor() {
    this.model = typeof HISTORY_TIMELINE !== "undefined" ? HISTORY_TIMELINE : null;
    this.root = document.getElementById("history-timeline");
    if (!this.model || !this.root || typeof d3 === "undefined") {
      return;
    }
    this.tree = document.location.pathname.split("/")[1];
    this.colors = [...d3.schemeTableau10, ...d3.schemeDark2];
    // (The times zoomed into, [from, to) each, the current one last.)
    this.zooms = [];
    this.render();
  }

  /**
   * How to group `count` points from `from` to `to` (Dates) on `plotWidth`
   * pixels: null for a marker each, if they fit, or else the finest of days,
   * weeks, months, quarters and years whose markers are far enough apart.
   */
  grouping(count, from, to, plotWidth) {
    if (count * MIN_MARKER_SPACING <= plotWidth) {
      return null;
    }
    const span = Math.max(to - from, DAY_MS);
    const quarter = d3.utcMonth.every(3);
    const units = [
      { name: "day", interval: d3.utcDay, ms: DAY_MS, label: d => d3.utcFormat("%Y-%m-%d")(d) },
      { name: "week", interval: d3.utcWeek, ms: 7 * DAY_MS, label: d => `the week of ${d3.utcFormat("%Y-%m-%d")(d)}` },
      { name: "month", interval: d3.utcMonth, ms: 30.44 * DAY_MS, label: d => d3.utcFormat("%Y-%m")(d) },
      { name: "quarter", interval: quarter, ms: 91.31 * DAY_MS, label: d => `${d.getUTCFullYear()} Q${Math.floor(d.getUTCMonth() / 3) + 1}` },
      { name: "year", interval: d3.utcYear, ms: 365.25 * DAY_MS, label: d => `${d.getUTCFullYear()}` },
    ];
    return units.find(unit => plotWidth * unit.ms / span >= MIN_MARKER_SPACING) ??
      units[units.length - 1];
  }

  /** Zoom into [from, to), or out a step (if null). */
  zoom(range) {
    if (range) {
      this.zooms.push(range);
    } else {
      this.zooms.pop();
    }
    this.render();
  }

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

  /** What the lines are, for the axis. */
  measure() {
    switch (this.model.measure) {
      case "value":
        return "Value";
      case "values":
        return "Distinct values";
      default:
        return "Matches";
    }
  }

  /**
   * A point's tooltip: its date (when it landed, with its commit date if
   * that was another day), revision, changes, and message.
   */
  tooltip(point) {
    const header = point.header.replace(/<[^>]*>/g, "");
    const changes = [
      ...point.added.map(value => `+ ${value}`),
      ...point.removed.map(value => `− ${value}`),
    ];
    const date = point.committed ? `${point.date} (committed ${point.committed})` : point.date;
    return [`${date} ${point.rev.slice(0, 12)}: ${point.value}`, ...changes, header].join("\n");
  }

  /**
   * A group's tooltip: its time, how many revisions, the value before and
   * after them, how many values they added and removed, and the first few of
   * them.
   */
  groupTooltip(label, points, before) {
    const after = points[points.length - 1].value;
    const added = points.reduce((n, point) => n + point.added.length, 0);
    const removed = points.reduce((n, point) => n + point.removed.length, 0);
    const lines = [
      `${label}: ${points.length} revisions, ${before} → ${after} (+${added} −${removed})`,
      ...points.slice(0, MAX_GROUP_TOOLTIP_REVISIONS).map(point => {
        const header = point.header.replace(/<[^>]*>/g, "");
        return `${point.date} ${point.rev.slice(0, 12)}: ${point.value} ` +
          `(+${point.added.length} −${point.removed.length}) ${header}`;
      }),
    ];
    if (points.length > MAX_GROUP_TOOLTIP_REVISIONS) {
      lines.push(`… and ${points.length - MAX_GROUP_TOOLTIP_REVISIONS} more`);
    }
    lines.push("(Click to zoom in.)");
    return lines.join("\n");
  }

  render() {
    this.root.replaceChildren();
    const series = this.model.series.filter(s => s.points.length);
    if (!series.length) {
      this.root.append(this.el("p", "history-timeline-empty", "No changes to chart."));
      return;
    }
    if (series.length > 1 || series[0].label) {
      const legend = this.el("ul", "history-timeline-legend");
      series.forEach((s, index) => {
        const item = this.el("li");
        const swatch = this.el("span", "history-timeline-swatch");
        swatch.style.backgroundColor = this.colors[index % this.colors.length];
        item.append(swatch, this.el("code", null, s.label || "all files"));
        legend.append(item);
      });
      this.root.append(legend);
    }

    // (Zoomed into a time: its buttons.)
    const zoom = this.zooms[this.zooms.length - 1];
    if (zoom) {
      const day = d3.utcFormat("%Y-%m-%d");
      const bar = this.el("div", "history-timeline-zoom");
      bar.append(this.el("span", null, `${day(zoom[0])} to ${day(new Date(zoom[1] - DAY_MS))} `));
      const out = this.el("button", "history-timeline-zoom-out", "Zoom out");
      out.addEventListener("click", () => this.zoom(null));
      bar.append(out);
      if (this.zooms.length > 1) {
        const all = this.el("button", "history-timeline-zoom-all", "Whole history");
        all.addEventListener("click", () => {
          this.zooms = [];
          this.render();
        });
        bar.append(" ", all);
      }
      this.root.append(bar);
    }

    const width = Math.max(600, this.root.clientWidth || 900);
    const height = 320;
    const margin = { top: 20, right: 16, bottom: 28, left: 64 };
    const plotWidth = width - margin.left - margin.right;
    const lines = series.map((s, index) => {
      const first = new Date(s.points[0].date);
      // (Its value before its first change, a little before it, so the step
      // shows; and its last value until now.)
      const points = [
        { date: new Date(first - 7 * DAY_MS), value: s.start },
        ...s.points.map(point => ({ date: new Date(point.date), value: point.value, point })),
        { date: new Date(), value: s.points[s.points.length - 1].value },
      ].filter(p => Number.isFinite(p.value));
      return { series: s, index, points };
    });
    const [from, to] = zoom ?? d3.extent(lines.flatMap(line => line.points.map(p => p.date)));
    const shown = p => !zoom || (p.date >= from && p.date < to);
    // (The values in the time, and, zoomed, those carried into it.)
    const values = lines.flatMap(line => {
      const before = line.points.filter(p => p.date < from).pop();
      return [...(zoom && before ? [before] : []), ...line.points.filter(shown)].map(p => p.value);
    });
    const x = d3.scaleUtc().domain([from, to]).range([margin.left, width - margin.right]);
    const [low, high] = zoom ? d3.extent(values) : [Math.min(0, ...values), Math.max(1, ...values)];
    const y = d3.scaleLinear()
      .domain(low == high ? [low - 1, high + 1] : [low, high])
      .nice()
      .range([height - margin.bottom, margin.top]);

    const svg = d3.select(this.root).append("svg")
      .attr("class", "history-timeline-svg")
      .attr("width", width)
      .attr("height", height)
      .attr("role", "img")
      .attr("aria-label", `${this.measure()} over time`);
    svg.append("defs").append("clipPath")
      .attr("id", "history-timeline-clip")
      .append("rect")
      .attr("x", margin.left)
      .attr("y", 0)
      .attr("width", plotWidth)
      .attr("height", height);
    svg.append("g")
      .attr("transform", `translate(0,${height - margin.bottom})`)
      .call(d3.axisBottom(x).ticks(Math.min(10, width / 90)));
    svg.append("g")
      .attr("transform", `translate(${margin.left},0)`)
      .call(d3.axisLeft(y).ticks(6));
    svg.append("text")
      .attr("class", "history-timeline-axis-label")
      .attr("x", margin.left)
      .attr("y", margin.top - 6)
      .text(this.measure());

    const line = d3.line()
      .x(p => x(p.date))
      .y(p => y(p.value))
      .curve(d3.curveStepAfter);
    const count = lines.reduce((n, line) => n + line.points.filter(p => p.point && shown(p)).length, 0);
    const grouping = this.grouping(count, from, to, plotWidth);
    if (grouping) {
      this.root.append(this.el(
        "p", "history-timeline-note",
        `${count} revisions: a bigger marker is a ${grouping.name}'s (hover for them, click to zoom in).`));
    }
    for (const { points, index } of lines) {
      const color = this.colors[index % this.colors.length];
      svg.append("path")
        .attr("class", "history-timeline-line")
        .attr("fill", "none")
        .attr("stroke", color)
        .attr("stroke-width", 2)
        .attr("clip-path", "url(#history-timeline-clip)")
        .attr("d", line(points));
      const markers = svg.append("g").attr("class", "history-timeline-markers");
      // (The revisions in the time, in groups of one, or of `grouping`'s
      // unit, each with the value before it.)
      const groups = [];
      let before = points[0].value;
      for (const p of points) {
        if (p.point && shown(p)) {
          const key = grouping ? +grouping.interval.floor(p.date) : groups.length;
          const last = groups[groups.length - 1];
          if (last && last.key == key) {
            last.points.push(p);
          } else {
            groups.push({ key, before, points: [p] });
          }
        }
        before = p.value;
      }
      for (const group of groups) {
        const last = group.points[group.points.length - 1];
        if (group.points.length == 1) {
          const link = markers.append("a")
            .attr("class", "history-timeline-marker")
            .attr("href", `/${this.tree}/commit/${last.point.rev}`)
            .attr("data-rev", last.point.rev);
          link.append("title").text(this.tooltip(last.point));
          link.append("circle")
            .attr("cx", x(last.date))
            .attr("cy", y(last.value))
            .attr("r", 4)
            .attr("fill", color);
          continue;
        }
        const start = new Date(group.key);
        const range = [start, grouping.interval.offset(start, 1)];
        const marker = markers.append("g")
          .attr("class", "history-timeline-group")
          .attr("role", "button")
          .attr("tabindex", 0)
          .attr("data-count", group.points.length)
          .attr("aria-label", `${grouping.label(start)}: ${group.points.length} revisions`)
          .on("click", () => this.zoom(range))
          .on("keydown", event => {
            if (event.key == "Enter" || event.key == " ") {
              event.preventDefault();
              this.zoom(range);
            }
          });
        marker.append("title").text(
          this.groupTooltip(grouping.label(start), group.points.map(p => p.point), group.before));
        marker.append("circle")
          .attr("cx", x(last.date))
          .attr("cy", y(last.value))
          .attr("r", Math.min(9, 3 + Math.sqrt(group.points.length)))
          .attr("fill", color);
      }
    }
  }
})();
