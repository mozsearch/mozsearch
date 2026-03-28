var CoverageGraph = new (class CoverageGraph {
  // Keep in sync with .cov-percentage-* in CSS
  COLOR_SCALE = [
    "#d91a47",
    "#c62828",
    "#d32f2f",
    "#e53935",
    "#f57c00",
    "#fbc02d",
    "#c0ca33",
    "#7cb342",
    "#43a047",
    "#2e7d32",
    "#00c853",
  ];

  colorForRevision(revision) {
    return this.COLOR_SCALE[Math.round(revision.percentage / 10)];
  }

  constructor() {
    this.sparkline = document.querySelector("#coverage-sparkline");
    this.graph = document.querySelector("#coverage-graph");
    this.history = document.querySelector("#coverage-history");
    const dateEl = document.querySelector("#rev-id time");

    if (!this.sparkline || !this.graph || !this.history || !dateEl) {
      return;
    }

    this.commitDate = new Date(dateEl.dateTime);

    const items = this.history.querySelectorAll(".coverage-history-item");
    this.revisions = Array.from(items).map(item => {
      const rev = item.dataset.rev
      const percentage = parseFloat(item.dataset.percentage)
      const date = new Date(item.querySelector("time").dateTime);
      const link = item.querySelector("a").href;

      return {rev, percentage, date, link};
    });

    if (this.revisions.length < 1) {
      return;
    }

    const minDate = this.commitDate < this.revisions[0].date ? this.commitDate : this.revisions[0].date;
    const maxDate = this.commitDate > this.revisions[this.revisions.length - 1].date ? this.commitDate : this.revisions[this.revisions.length - 1].date;
    this.xDomain = [minDate, maxDate];
    this.yDomain = [0, 100];

    this.drawSparkLine();
  }

  makeSvg(container, width, height, padding) {
    d3.select(container).selectAll("*").remove();

    const svg = d3.select(container)
      .append("svg")
      .attr("width", width)
      .attr("height", height)

    const formatDay = d3.utcFormat("%a %d"),
      formatMonth = d3.utcFormat("%B"),
      formatYear = d3.utcFormat("%Y");

    function multiFormat(date) {
      return (
        d3.utcMonth(date) < date ? formatDay
        : d3.utcYear(date) < date ? formatMonth
        : formatYear
      )(date);
    }

    const x = d3.scaleUtc()
      .domain(this.xDomain)
      .range([padding, width - padding]);

    x.tickFormat(multiFormat)

    const y = d3.scaleLinear()
      .domain(this.yDomain)
      .range([height - padding, padding]);

    return [svg, x, y]
  }

  makeLineGradient(svg, y, gradientId) {
    let defs = svg.select("defs");
    if (defs.empty()) {
      defs = svg.append("defs");
    }

    defs.append("linearGradient")
      .attr("id", gradientId)
      .attr("gradientUnits", "userSpaceOnUse")
      .attr("x1", 0)
      .attr("y1", y(this.yDomain[0]))
      .attr("x2", 0)
      .attr("y2", y(this.yDomain[1]))
      .selectAll("stop")
      .data(this.COLOR_SCALE)
      .join("stop")
      .attr("offset", (_color, i) => i / (this.COLOR_SCALE.length - 1))
      .attr("stop-color", color => color);
  }

  drawCommitDateMarker(svg, x, y) {
    const color = "#7a7a7a";
    const strokeWidth = 2;

    const path = d3.path();
    path.moveTo(x(this.commitDate), y(this.yDomain[0]))
    path.lineTo(x(this.commitDate), y(this.yDomain[1]));;

    svg.append("path")
      .attr("fill", "none")
      .attr("stroke", color)
      .attr("stroke-width", strokeWidth)
      .attr("stroke-linecap", "round")
      .attr("stroke-linejoin", "round")
      .attr("d", path);
  }

  drawLine(svg, x, y, gradientId) {
    const strokeWidth = 2;

    this.makeLineGradient(svg, y, gradientId);

    this.drawCommitDateMarker(svg, x, y);

    const line = d3.line()
      .x(revision => x(revision.date))
      .y(revision => y(revision.percentage))
      .curve(d3.curveMonotoneX);

    svg.append("path")
      .datum(this.revisions)
      .attr("fill", "none")
      .attr("stroke", `url(#${gradientId})`)
      .attr("stroke-width", strokeWidth)
      .attr("stroke-linecap", "round")
      .attr("stroke-linejoin", "round")
      .attr("d", line);
  }

  drawSparkLine() {
    if (this.revisions.length < 2) {
      return;
    }

    const width = 100;
    const height = 18;
    const padding = 0;

    const [svg, x, y] = this.makeSvg(this.sparkline, width, height, padding);
    this.drawLine(svg, x, y, "coverage-sparkline-gradient");
  }

  drawAxes(svg, x, y, height, padding) {
    const xAxis = svg.append("g")
      .attr("transform", `translate(0, ${height - padding})`)
      .call(d3.axisBottom(x));

    const yAxis = svg.append("g")
      .attr("transform", `translate(${padding}, 0)`)
      .call(d3.axisLeft(y));
  }

  drawArea(svg, x, y) {
    const areaColor = "#7a7a7a";
    const areaOpacity = 0.12;

    const area = d3.area()
      .x(revision => x(revision.date))
      .y0(y(0))
      .y1(revision => y(revision.percentage))
      .curve(d3.curveMonotoneX);

    svg.append("path")
      .datum(this.revisions)
      .attr("fill", areaColor)
      .attr("opacity", areaOpacity)
      .attr("d", area);
  }

  drawDots(svg, x, y) {
    const dotRadius = 2;

    const dots = svg.append("g")
      .selectAll("a")
      .data(this.revisions)
      .join("a")
      .attr("href", revision => revision.link)
      .attr("aria-labelledby", revision => `coverage-graph-tooltip-${revision.rev}`);

    const tooltips = dots.append("g")
      .attr("id", revision => `coverage-graph-tooltip-${revision.rev}`)
      .classed("tooltip", true);

    tooltips.append("rect")
      .attr("x", revision => Math.min(Math.max(x(revision.date), x(this.xDomain[0]) + 25), x(this.xDomain[1]) - 25))
      .attr("y", revision => y(revision.percentage));

    tooltips.append("text")
      .attr("x", revision => Math.min(Math.max(x(revision.date), x(this.xDomain[0]) + 25), x(this.xDomain[1]) - 25))
      .attr("y", revision => y(revision.percentage))
      .text(revision => `${revision.date.toISOString().split("T")[0]}: ${Math.round(revision.percentage)} %`);

    dots.append("circle")
      .attr("fill", revision => this.colorForRevision(revision))
      .attr("stroke", revision => this.colorForRevision(revision))
      .attr("cx", revision => x(revision.date))
      .attr("cy", revision => y(revision.percentage))
      .attr("r", dotRadius);
  }

  drawGraph() {
    const width = this.graph.getBoundingClientRect().width;
    const height = 300;
    const padding = 30;

    const [svg, x, y] = this.makeSvg(this.graph, width, height, padding);

    this.drawAxes(svg, x, y, height, padding);
    this.drawArea(svg, x, y);
    this.drawLine(svg, x, y, "coverage-graph-gradient");
    this.drawDots(svg, x, y);

    let hovered = null;
    svg.
      on("mousemove", (event) => {
        let closest = null;
        let distance = null;
        for (const revision of svg.selectAll("a")) {
          let x = revision.querySelector("circle").cx.animVal.value;
          let thisDistance = Math.abs(x - event.offsetX);
          if (distance === null || thisDistance <= distance) {
            closest = revision;
            distance = thisDistance;
          }
        }

        if (hovered !== null) {
          hovered.classList.remove("hovered");
        }
        hovered = closest;
        if (hovered !== null) {
          closest.classList.add("hovered");
        }
      })
      .on("mouseleave", () => {
        if (hovered !== null) {
          hovered.classList.remove("hovered");
        }
        hovered = null;
      })
      .on("click", (event) => {
        if (hovered !== null) {
          window.location = hovered.attributes.href.value;
        }
      })
  }

  open() {
    this.graph.showModal();
    this.drawGraph();
    d3.select(window).on('resize.updatesvg', () => this.drawGraph());
  }
})();
