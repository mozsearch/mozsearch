/**
 * A history grep's timeline (`text-timeline:`, `re-timeline:`; see `Timeline`
 * in tools/src/hyperblame/history_grep.rs): a step line per series (the
 * matches' count over the files, each file's captured number, or how many
 * distinct values the matches captured), with a marker at each revision
 * changing it, whose tooltip says what it added and removed, and which links
 * to the revision.  The table below the chart (history_grep_root.liquid) has
 * the same changes.
 */
var HistoryTimeline = new (class HistoryTimeline {
  constructor() {
    this.model = typeof HISTORY_TIMELINE !== "undefined" ? HISTORY_TIMELINE : null;
    this.root = document.getElementById("history-timeline");
    if (!this.model || !this.root || typeof d3 === "undefined") {
      return;
    }
    this.tree = document.location.pathname.split("/")[1];
    this.colors = [...d3.schemeTableau10, ...d3.schemeDark2];
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

    const width = Math.max(600, this.root.clientWidth || 900);
    const height = 320;
    const margin = { top: 20, right: 16, bottom: 28, left: 64 };
    const lines = series.map((s, index) => {
      const first = new Date(s.points[0].date);
      // (Its value before its first change, a little before it, so the step
      // shows; and its last value until now.)
      const points = [
        { date: new Date(first - 7 * 24 * 3600 * 1000), value: s.start },
        ...s.points.map(point => ({ date: new Date(point.date), value: point.value, point })),
        { date: new Date(), value: s.points[s.points.length - 1].value },
      ].filter(p => Number.isFinite(p.value));
      return { series: s, index, points };
    });
    const dates = lines.flatMap(line => line.points.map(p => p.date));
    const values = lines.flatMap(line => line.points.map(p => p.value));
    const x = d3.scaleUtc().domain(d3.extent(dates)).range([margin.left, width - margin.right]);
    const y = d3.scaleLinear()
      .domain([Math.min(0, ...values), Math.max(1, ...values)])
      .nice()
      .range([height - margin.bottom, margin.top]);

    const svg = d3.select(this.root).append("svg")
      .attr("class", "history-timeline-svg")
      .attr("width", width)
      .attr("height", height)
      .attr("role", "img")
      .attr("aria-label", `${this.measure()} over time`);
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
    for (const { points, index } of lines) {
      const color = this.colors[index % this.colors.length];
      svg.append("path")
        .attr("class", "history-timeline-line")
        .attr("fill", "none")
        .attr("stroke", color)
        .attr("stroke-width", 2)
        .attr("d", line(points));
      const markers = svg.append("g").attr("class", "history-timeline-markers");
      for (const p of points.filter(p => p.point)) {
        const link = markers.append("a")
          .attr("class", "history-timeline-marker")
          .attr("href", `/${this.tree}/commit/${p.point.rev}`)
          .attr("data-rev", p.point.rev);
        link.append("title").text(this.tooltip(p.point));
        link.append("circle")
          .attr("cx", x(p.date))
          .attr("cy", y(p.value))
          .attr("r", 4)
          .attr("fill", color);
      }
    }
  }
})();
