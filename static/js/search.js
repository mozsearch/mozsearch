var Dxr = new (class Dxr {
  constructor() {
    let constants = document.getElementById("data");

    this.wwwRoot = constants.getAttribute("data-root");
    this.baseUrl = location.protocol + "//" + location.host;
    this.icons = this.wwwRoot + "/static/icons/";
    this.views = this.wwwRoot + "/static/templates";
    this.searchUrl = constants.getAttribute("data-search");
    this.tree = constants.getAttribute("data-tree");
    this.timeouts = {
      search: 300,
      // We start the history timeout after the search updates (i.e., after
      // timeouts.search has elapsed).
      history: 2000 - 300,
    };
    this.searchBox = document.getElementById("search-box");

    // TODO(emilio): Maybe these should be their own web component or
    // something.
    this.fields = {
      query: document.getElementById("query"),
      path: document.getElementById("path"),
      caseSensitive: document.getElementById("case"),
      regexp: document.getElementById("regexp"),
    };
    this.bubbles = {
      query: document.getElementById("query-bubble"),
      path: document.getElementById("path-bubble"),
    };

    this.startSearchTimer = null;
    // The timer to move to the next url.
    this.historyTimer = null;
    // The controller to allow aborting a fetch().
    this.fetchController = null;

    window.addEventListener("pageshow", () =>
      this.initFormFromLocalStorageOrUrl()
    );
    // FIXME: This reloads the page when you navigate to #lineno.
    window.addEventListener("popstate", () => window.location.reload(), {
      once: true,
    });

    this.fields.query.addEventListener("input", () => this.startSearchSoon());
    this.fields.path.addEventListener("input", () => this.startSearchSoon());
    this.fields.regexp.addEventListener("change", () => this.startSearch());
    this.fields.caseSensitive.addEventListener("change", event => {
      window.localStorage.setItem("caseSensitive", event.target.checked);
      this.startSearch();
    });
    this.initFormFromLocalStorageOrUrl();
  }

  cancel(cancelFetch = true) {
    if (this.startSearchTimer) {
      clearTimeout(this.startSearchTimer);
      this.startSearchTimer = null;
    }
    if (this.historyTimer) {
      clearTimeout(this.historyTimer);
      this.historyTimer = null;
    }
    if (cancelFetch && this.fetchController) {
      this.fetchController.abort();
      this.fetchController = null;
    }
  }

  startSearchSoon() {
    this.cancel(/* cancelFetch = */ false);
    this.startSearchTimer = setTimeout(() => {
      this.startSearchTimer = null;
      this.startSearch();
    }, this.timeouts.search);
  }

  async startSearch() {
    this.cancel();

    let query = this.fields.query.value;
    let path = this.fields.path.value.trim();

    if (!query.length && !path.length) {
      return this.hideBubbles();
    }

    if (query.length < 3 && path.length < 3) {
      return this.showBubble(
        "info",
        "Enter at least 3 characters to do a search.",
        query.length ? "query" : "path"
      );
    }

    this.hideBubbles();

    let url = new URL(this.searchUrl, window.location);
    url.searchParams.set("q", this.fields.query.value);
    url.searchParams.set("path", this.fields.path.value.trim());
    url.searchParams.set("case", this.fields.caseSensitive.checked);
    url.searchParams.set("regexp", this.fields.regexp.checked);
    let controller = new AbortController();

    this.fetchController = controller;

    this.searchBox.classList.add("in-progress");
    let results;
    try {
      let response = await fetch(url.href, {
        headers: {
          Accept: "application/json",
        },
        signal: this.fetchController.signal,
      });
      results = await response.json();
      if (!response.ok) {
        return this.showBubble(results.error_level, results.error_html);
      }
    } catch (error) {
      if (controller.signal.aborted) {
        // This fetch was cancelled in order to do a new query, nothing to do
        // here.
        return;
      }
      return this.showBubble("error", "An error occurred. Please try again.");
    } finally {
      this.searchBox.classList.remove("in-progress");
    }

    populateResults(results, false, false);
    this.historyTimer = setTimeout(() => {
      this.historyTimer = null;
      window.history.pushState({}, "", url.href);
    }, this.timeouts.history);
  }

  initFormFromLocalStorageOrUrl() {
    let url = new URL(location.href);
    let params = url.searchParams;

    // If the `case` param is in the URL, use its boolean value, so that the
    // checkbox reflects what's in the URL. If the `case` param is not in the
    // URL and this is in fact a search URL, then the search is implicitly
    // case-insensitive, so ensure the checkbox reflects that. Don't update
    // the localStorage value in either of these cases, because the user
    // may just have received a link from somebody else and we don't want to
    // update the user's saved defaults. The saved defaults *only* get updated
    // when the user explicitly clicks on the checkbox and the change event
    // listener triggers.
    // Finally, if we're not in a search already, have the checkbox reflect
    // the saved default from localStorage.
    let caseSensitive = params.get("case");
    if (caseSensitive) {
      caseSensitive = caseSensitive === "true";
    } else if (params.get("q")) {
      caseSensitive = false;
    } else {
      caseSensitive = window.localStorage.getItem("caseSensitive") === "true";
    }
    this.fields.caseSensitive.checked = caseSensitive;

    this.fields.regexp.checked = params.get("regexp") === "true";

    let query = params.get("q");
    if (query) {
      this.fields.query.value = query;
    }

    let path = params.get("path");
    if (path) {
      this.fields.path.value = path;
    }
  }

  // Hang an advisory message off the search field.
  // @param {string} level - The seriousness: 'info', 'warning', or 'error'
  // @param {string} html - The HTML message to be displayed
  // @param {string} which - 'query' or 'path', defaults to the focused element
  // or 'query' otherwise.
  showBubble(level, html, which) {
    if (!which) {
      which = document.activeElement == this.fields.path ? "path" : "query";
    }
    let other = which == "path" ? "query" : "path";
    this.hideBubble(other);

    let bubble = this.bubbles[which];
    bubble.classList.remove("error");
    bubble.classList.remove("warning");
    bubble.classList.remove("info");
    bubble.classList.add(level);
    bubble.innerHTML = html;

    // TODO(emilio): Old code animated the bubble.
    bubble.style.display = "block";
  }

  hideBubble(which) {
    // TODO(emilio): Old code animated the bubble.
    this.bubbles[which].style.display = "none";
  }

  hideBubbles() {
    for (let kind in this.bubbles) {
      this.hideBubble(kind);
    }
  }
})();

function hashString(string) {
  let hash = 0;
  if (string.length == 0) {
    return hash;
  }
  for (let i = 0; i < string.length; i++) {
    let char = string.charCodeAt(i);
    hash = (hash << 5) - hash + char;
    hash = hash & hash; // Convert to 32bit integer
  }
  return hash;
}

function classOfResult(pathkind, qkind, isContext) {
  var klass = pathkind + ":" + qkind;
  let cssClass = "EXPANDO" + hashString(klass);
  if (isContext) {
    cssClass += ` ${isContext}-context-line`;
  }
  return cssClass;
}

function onExpandoClick(event) {
  let target = event.target;
  let wasOpen = target.classList.contains("open");
  let elements = document.querySelectorAll(
    "." + target.getAttribute("data-klass")
  );
  for (let element of elements) {
    element.style.display = wasOpen ? "none" : "";
  }
  target.classList.toggle("open");
  target.innerHTML = wasOpen ? "&#9654;" : "&#9660;";
}

var populateEpoch = 0;
function populateResults(data, full, jumpToSingle) {
  populateEpoch++;

  var title = data["*title*"];
  if (title) {
    delete data["*title*"];
    document.title = title + " - mozsearch";
  }
  var timed_out = data["*timedout*"];
  delete data["*timedout*"];

  window.scrollTo(0, 0);

  function makeURL(path) {
    return "/" + Dxr.tree + "/source/" + path;
  }

  function chooseIcon(path) {
    var suffix = path.lastIndexOf(".");
    if (suffix == -1) {
      return "unknown";
    }
    suffix = path.slice(suffix + 1);
    return (
      {
        cpp: "cpp",
        h: "h",
        c: "c",
        mm: "mm",
        js: "js",
        jsm: "js",
        py: "py",
        ini: "conf",
        sh: "sh",
        txt: "txt",
        xml: "xml",
        xul: "ui",
        java: "java",
        in: "txt",
        html: "html",
        png: "image",
        gif: "image",
        svg: "svg",
        build: "build",
        json: "js",
        css: "css",
      }[suffix] || "unknown"
    );
  }

  function renderPath(pathkind, qkind, fileResult) {
    var klass = classOfResult(pathkind, qkind);

    var html = "";
    html += "<tr class='result-head " + klass + "'>";
    html +=
      "<td class='left-column'><div class='" +
      chooseIcon(fileResult.path) +
      " icon-container'></div></td>";

    // This span exists for a11y reasons.  See bug 1558691 but the core idea is:
    // - It's fine (and good!) to upgrade this to an explicit h3 in the future.
    //   We should always favor semantic HTML over use of divs/spans.
    // - At the current moment we're just introducing the span because the
    //   styling fallout is potentially more than we want to get into.
    html += `<td><span role="heading" aria-level="3">`;

    var elts = fileResult.path.split("/");
    var pathSoFar = "";
    for (var i = 0; i < elts.length; i++) {
      if (i != 0) {
        html += "<span class='path-separator'>/</span>";
      }

      var elt = elts[i];
      pathSoFar += elt;
      html += "<a href='" + makeURL(pathSoFar) + "'>" + elt + "</a>";
      pathSoFar += "/";
    }

    html += "</span></td>";
    html += "</tr>";

    return html;
  }

  function renderSingleSearchResult(pathkind, qkind, file, line, isContext) {
    var [start, end] = line.bounds || [0, 0];
    var before = line.line.slice(0, start).replace(/^\s+/, "");
    var middle = line.line.slice(start, end);
    var after = line.line.slice(end).replace(/\s+$/, "");

    var klass = classOfResult(pathkind, qkind, isContext);
    var html = "";
    html += "<tr class='" + klass + "'>";
    html +=
      "<td class='left-column'><a href='" +
      makeURL(file.path) +
      "#" +
      line.lno +
      "'>" +
      line.lno +
      "</a></td>";
    html += "<td><a href='" + makeURL(file.path) + "#" + line.lno + "'>";

    function escape(s) {
      return s.replace(/&/gm, "&amp;").replace(/</gm, "&lt;");
    }

    html += "<code>";
    html += escape(before);
    html += "<b>" + escape(middle) + "</b>";
    html += escape(after);
    html += "</code>";

    html += "</a>";

    if (line.context) {
      var inside = line.context;
      if (line.contextsym) {
        var url = `/${Dxr.tree}/search?q=symbol:${encodeURIComponent(
          line.contextsym
        )}&redirect=false`;
        inside = "<a href='" + url + "'>" + line.context + "</a>";
      }
      html +=
        " <span class='result-context'>// found in <code>" +
        inside +
        "</code></span>";
    }

    html += "</td>";
    html += "</tr>";

    return html;
  }

  // Accumulate the total number of results for our initial summary and to
  // support our "automatically go to a single result" UX optimization below.
  var count = 0;
  for (var pathkind in data) {
    for (var qkind in data[pathkind]) {
      for (var k = 0; k < data[pathkind][qkind].length; k++) {
        var path = data[pathkind][qkind][k];
        count += path.lines.length;
      }
    }
  }

  // Accumulate the total number of files to support our "automatically go to a
  // single result" UX optimization below.
  var fileCount = 0;
  for (var pathkind in data) {
    for (var qkind in data[pathkind]) {
      fileCount += data[pathkind][qkind].length;
    }
  }

  // If there's only a single result, redirect ourselves there directly.
  if (jumpToSingle && fileCount == 1 && count <= 1) {
    var pathkind = Object.keys(data)[0];
    var qkind = Object.keys(data[pathkind])[0];
    var file = data[pathkind][qkind][0];
    var path = file.path;

    if (count == 1) {
      var line = file.lines[0];
      var lno = line.lno;
      window.location = `/${Dxr.tree}/source/${path}#${lno}`;
    } else {
      window.location = `/${Dxr.tree}/source/${path}`;
    }
    return;
  }

  // If no data is returned, inform the user.
  let container = document.getElementById("content");
  container.innerHTML = "";

  let timeoutWarning = timed_out
    ? "<div>Warning: results may be incomplete due to server-side search timeout!</div>"
    : "";

  if (!fileCount) {
    container.insertAdjacentHTML(
      "beforeend",
      "<span>No results for current query.</span>"
    );
    if (timeoutWarning) {
      container.insertAdjacentHTML("beforeend", timeoutWarning);
    }
  } else {
    if (count) {
      container.insertAdjacentHTML(
        "beforeend",
        `<div>Number of results: ${count} (maximum is 1000)</div>`
      );
    }
    if (timeoutWarning) {
      container.insertAdjacentHTML("beforeend", timeoutWarning);
    }

    var table = document.createElement("table");
    table.className = "results";

    container.appendChild(table);

    var counter = 0;

    var pathkindNames = {
      normal: null,
      test: "Test files",
      generated: "Generated code",
      thirdparty: "Third-party code",
    };

    var html = "";
    // Loop over normal/test/generated/thirdparty "pathkind"s
    for (var pathkind in data) {
      var pathkindName = pathkindNames[pathkind];
      if (pathkindName) {
        html += "<tr><td>&nbsp;</td></tr>";
        html +=
          "<tr><td class='section'>§</td><td><div class='result-pathkind'>" +
          pathkindName +
          "</div></td></tr>";
      }

      // Loop over definition/declaration/use/etc. "qkind"s
      var qkinds = Object.keys(data[pathkind]);
      for (var qkind in data[pathkind]) {
        if (data[pathkind][qkind].length) {
          html += "<tr><td>&nbsp;</td></tr>";

          html += "<tr><td class='left-column'>";
          html +=
            "<div class='expando open' data-klass='" +
            classOfResult(pathkind, qkind) +
            "'>&#9660;</div>";
          html += "</td>";

          html += "<td><h2 class='result-kind'>" + qkind + "</h2></td></tr>";
        }

        // Loop over the files with hits.
        for (var i = 0; i < data[pathkind][qkind].length; i++) {
          var file = data[pathkind][qkind][i];

          if (counter > 100 && !full) {
            break;
          }

          html += renderPath(pathkind, qkind, file);

          file.lines.map(function (line) {
            counter++;
            if (counter > 100 && !full) {
              return;
            }

            if (line.context_before) {
              let lineDelta = -line.context_before.length;
              for (const lineStr of line.context_before) {
                html += renderSingleSearchResult(
                  pathkind, qkind, file,
                  { lno: line.lno + lineDelta, line: lineStr }, 'before');
                lineDelta++;
              }
            }
            html += renderSingleSearchResult(pathkind, qkind, file, line);
            if (line.context_after) {
              let lineDelta = 1;
              for (const lineStr of line.context_after) {
                html += renderSingleSearchResult(
                  pathkind, qkind, file,
                  { lno: line.lno + lineDelta, line: lineStr }, 'after');
                lineDelta++;
              }
            }
          });
        }
      }
    }

    table.innerHTML = html;

    for (let element of document.querySelectorAll(".expando")) {
      element.addEventListener("click", onExpandoClick);
    }

    if (counter > 100 && !full) {
      var epoch = populateEpoch;
      setTimeout(function () {
        if (populateEpoch == epoch) {
          populateResults(data, true, false);
        }
      }, 750);
    }
  }
}

window.showSearchResults = function (results) {
  var jumpToSingle = window.location.search.indexOf("&redirect=false") == -1;
  populateResults(results, true, jumpToSingle);
};

/**
 * Adds a leading 0 to numbers less than 10 and greater that 0
 *
 * @param int number The number to test against
 *
 * return Either the original number or the number prefixed with 0
 */
function addLeadingZero(number) {
  return number <= 9 || number === 0 ? "0" + number : number;
}

/**
 * Converts string to new Date and returns a formatted date in the
 * format YYYY-MM-DD h:m
 * @param String dateString A date in string form.
 *
 */
function formatDate(dateString) {
  var fullDateTime = new Date(dateString),
    date =
      fullDateTime.getFullYear() +
      "-" +
      (fullDateTime.getMonth() + 1) +
      "-" +
      addLeadingZero(fullDateTime.getDate()),
    time =
      fullDateTime.getHours() + ":" + addLeadingZero(fullDateTime.getMinutes());

  return date + " " + time;
}

for (let element of document.querySelectorAll(".pretty-date")) {
  element.innerText = formatDate(element.getAttribute("data-datetime"));
}
