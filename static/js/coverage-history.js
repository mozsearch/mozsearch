// Get the coverage history items and convert them to an array of objects with the percent, date, and link
const items = document.querySelectorAll("#coverage-history .coverage-history-item");

const data = Array.from(items).map(item => {
    const percent = parseFloat(item.querySelector(".coverage-history-percentage").textContent)
    const date = item.querySelector("time").getAttribute("datetime");
    const link = item.querySelector("a").href;

    return {percent,date,link};
});

console.log(data);

/* Create a drawSparkline function */

function drawSparkLine(container,data,options = {}) {
    if(!data || data.length < 2) return;

    const width = options.width || 100;
    const height = options.height || 24;
    const strokeWidth = options.strokeWidth || 1.5;
    const color = options.color || "#4a90e2";

    const padding = strokeWidth + 2;
    const innerWidth = width - padding * 2;
    const innerHeight = height - padding * 2;

    const values = data.map(d => d.percent);
    const minY = Math.min(...values,0);
    const maxY = Math.max(...values,100);

    const range = maxY - minY;

    // const minY = 0;
    // const maxY = 100;

    const stepX = innerWidth / (data.length - 1);

    const points = data.map((data , index) => {
        const x = padding + index * stepX;
        const y = padding + innerHeight - ((data.percent - minY) / range) * innerHeight;

        return { x, y, date : data.date, percent : data.percent , link : data.link};

    });

    const pathD = points
        .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`)
        .join(" ");

    const svgNS = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(svgNS, "svg");
    svg.setAttribute("width", width);
    svg.setAttribute("height", height);
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.style.display = "block";
  
    const path = document.createElementNS(svgNS, "path");
    path.setAttribute("d", pathD);
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", color);
    path.setAttribute("stroke-width", strokeWidth);
    path.setAttribute("stroke-linecap", "round");
    path.setAttribute("stroke-linejoin", "round");
  
    svg.appendChild(path);

    points.forEach(point => {
        const circle = document.createElementNS(svgNS, "circle");
        circle.setAttribute("cx", point.x);
        circle.setAttribute("cy", point.y);
        circle.setAttribute("r", 2.8);
        circle.setAttribute("fill", color);
        circle.style.cursor = "pointer";

        // tooltip
        const title = document.createElementNS(svgNS,"title");
        title.textContent = `${point.date} : ${point.percent}%`;

        circle.appendChild(title);

        if (point.link){
            circle.addEventListener("click", () => {
                window.location.href = point.link;
            });
        }
        svg.appendChild(circle);
    });
  
    container.innerHTML = "";
    container.appendChild(svg);
  }

// Draw the sparkline for each element
const sparkLineElements = document.querySelectorAll(".coverage-sparkline");

sparkLineElements.forEach(element => {
    drawSparkLine(element, data, {
        width:500,
        height:20,
        strokeWidth:1.2,
        color: "#4a90e2"
    });
});

