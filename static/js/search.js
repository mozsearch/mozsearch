var Dxr = new (class Dxr {
  constructor() {
    let constants = document.getElementById("data");

    // This will usually be "/"
    this.wwwRoot = constants.getAttribute("data-root");
    // This will look like "mozilla-central"
    this.tree = constants.getAttribute("data-tree");
    this.baseUrl = location.protocol + "//" + location.host;
    // This will end up "/TREE/static/icons/"
    this.icons = this.wwwRoot + `${this.tree}/static/icons/`;
    // This will usually be "/TREE/search"
    this.searchUrl = constants.getAttribute("data-search");
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

    this.setupColSelector();

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

    // XXX hacky mechanism so that we only run this on pages with a "search"
    // header rather than a "query" header.  We should refactor this and the
    // general code listing generation so that we:
    // - pick search versus query based on Setting and can change by user
    //   choice even on any page.
    // - explicitly understand if it's operating in search or query mode.
    if (this.fields.path) {
      this.fields.query.addEventListener("input", () => this.startSearchSoon());
      this.fields.path.addEventListener("input", () => this.startSearchSoon());
      this.fields.regexp.addEventListener("change", () => this.startSearch());
      this.fields.caseSensitive.addEventListener("change", event => {
        window.localStorage.setItem("caseSensitive", event.target.checked);
        this.startSearch();
      });
      this.initFormFromLocalStorageOrUrl();
    }
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

    let url = this.constructURL();

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

    this.updateHistory(url);
  }

  constructURL() {
    let url = new URL(this.searchUrl, window.location);
    url.searchParams.set("q", this.fields.query.value);
    if (this.fields.path) {
      url.searchParams.set("path", this.fields.path.value.trim());
    }
    if (this.fields.caseSensitive) {
      url.searchParams.set("case", this.fields.caseSensitive.checked);
    }
    if (this.fields.regexp) {
      url.searchParams.set("regexp", this.fields.regexp.checked);
    }
    return url;
  }

  updateHistory(url) {
    this.historyTimer = setTimeout(() => {
      this.historyTimer = null;
      window.history.pushState({}, "", url.href);
    }, this.timeouts.history);
  }

  initFormFromLocalStorageOrUrl() {
    // XXX similar to in the constructor, we're using the path field as an
    // indication of the mode we're in, but we should clean this up.
    if (!this.fields.path) {
      return;
    }

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

  setupColSelector() {
    this.colSelector = document.querySelector("#symbol-tree-table-col-selector");
    if (!this.colSelector) {
      return;
    }
    this.symbolTreeTableList = document.querySelector("#symbol-tree-table-list");
    if (!this.symbolTreeTableList) {
      return;
    }

    const defaultCols = {
      name: true,
      type: false,
      line: true,
    };

    this.cols = {};
    for (const [key, defaultValue] of Object.entries(defaultCols)) {
      const node = document.querySelector("#col-show-" + key);
      node.addEventListener("change", () => {
        this.onColChange();
      });
      this.cols[key] = {
        node: node,
        currentValue: defaultValue,
        defaultValue: defaultValue,
      };
    }

    this.parseColQuery();
  }

  parseColQuery() {
    if (!this.colSelector) {
      return;
    }

    let query = this.fields.query.value;

    for (const m of query.matchAll(/(show|hide)-cols:([a-z,]+)/g)) {
      const show = m[1] == "show";
      const cols = m[2].split(/,/);

      for (const col of cols) {
        this.cols[col].currentValue = show;
      }
    }

    this.updateColCheckbox();
  }

  updateColCheckbox() {
    for (const [key, obj] of Object.entries(this.cols)) {
      obj.node.checked = obj.currentValue;
    }
  }

  onColChange() {
    for (const [key, obj] of Object.entries(this.cols)) {
      obj.currentValue = obj.node.checked;

      if (!obj.defaultValue) {
        this.symbolTreeTableList.classList.toggle("show-" + key, obj.currentValue);
      } else {
        this.symbolTreeTableList.classList.toggle("hide-" + key, !obj.currentValue);
      }
    }

    this.updateColQuery();
  }

  getShowCols() {
    const showCols = [];
    for (const [key, obj] of Object.entries(this.cols)) {
      if (obj.currentValue != obj.defaultValue && !obj.defaultValue) {
        showCols.push(key);
      }
    }
    return showCols.join(",");
  }

  getHideCols() {
    const hideCols = [];
    for (const [key, obj] of Object.entries(this.cols)) {
      if (obj.currentValue != obj.defaultValue && obj.defaultValue) {
        hideCols.push(key);
      }
    }
    return hideCols.join(",");
  }

  updateColQuery() {
    const showCols = this.getShowCols();
    const hideCols = this.getHideCols();

    let query = this.fields.query.value;
    query = query.replace(/ +(show|hide)-cols:([a-z,]+)/g, "");

    if (showCols) {
      query += " show-cols:" + showCols;
    }
    if (hideCols) {
      query += " hide-cols:" + hideCols;
    }

    this.fields.query.value = query;
    let url = this.constructURL();
    this.updateHistory(url);
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

  // We use to delete these fields from the data, but that's not compatible of
  // the current 2-pass rendering approach, so now the logic below knows to skip
  // any key that starts with a "*".
  var title = data["*title*"];
  if (title) {
    document.title = title + " - mozsearch";

    // Tell the title to webtest.
    document.dispatchEvent(new Event("titlechanged"));
  }
  var timed_out = data["*timedout*"];

  let limits_hit = data["*limits*"] || [];

  window.scrollTo(0, 0);

  function makeURL(path) {
    return "/" + Dxr.tree + "/source/" + path;
  }

  function makeSearchUrl(q) {
    return `/${Dxr.tree}/search?q=${encodeURIComponent(q)}`;
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
        m: "mm",
        mm: "mm",
        js: "js",
        jsm: "js",
        mjs: "js",
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
      "<td class='left-column'><div class='mimetype-icon-" +
      chooseIcon(fileResult.path) +
      " mimetype-floating-container'></div></td>";

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

  function escape(s) {
    return s.replace(/&/gm, "&amp;").replace(/</gm, "&lt;");
  }

  function renderSingleSearchResult(pathkind, qkind, file, line, isContext, hasContext) {
    var [start, end] = line.bounds || [0, 0];
    var before = line.line.slice(0, start);
    // Do not truncate off the leading whitespace if we're trying to present in context.
    if (!hasContext) {
      before = before.replace(/^\s+/, "");
    }
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

    html += "<code>";
    // in the context cases, we may only have an after.
    if (before) {
      html += escape(before);
    }
    if (middle) {
      html += "<b>" + escape(middle) + "</b>";
    }
    html += escape(after);
    html += "</code>";

    html += "</a>";

    if (line.context) {
      var inside = line.context;
      if (line.contextsym) {
        var url = `/${Dxr.tree}/search?q=symbol:${encodeURIComponent(
          line.contextsym
        )}&redirect=false`;
        inside = "<a href='" + url + "'>" + escape(line.context) + "</a>";
      }
      html +=
        " <span class='result-context'>// found in <code>" +
        inside +
        "</code></span>";
    }

    // Hacky attempt to provide a means of providing related searches.
    if (line.upsearch) {
      html += `<span class='result-upsearch'><a href="${makeSearchUrl(
        line.upsearch
      )}">Symbol Search This</a></span>`;
    }

    html += "</td>";
    html += "</tr>";

    return html;
  }

  // Accumulate the total number of results for our initial summary and to
  // support our "automatically go to a single result" UX optimization below.
  var count = 0;
  var tupledCounts = new Map();
  var pathkindCounts = new Map();
  for (var pathkind in data) {
    // Skip metadata fields.  The current idiom calls this method twice to
    // reduce initial above-the-folder rendering into place, which means that
    // we can't just delete these fields.
    if (pathkind.startsWith("*")) {
      continue;
    }
    let pathkindHits = 0;
    let pathkindFiles = 0;
    for (var qkind in data[pathkind]) {
      let qkindHitcount = 0;
      for (var k = 0; k < data[pathkind][qkind].length; k++) {
        var path = data[pathkind][qkind][k];
        // 0 lines implies this is a file, in which case just the bare file still
        // counts for our result count purposes and how router.py determined the
        // limits.
        count += (path.lines.length || 1);
        qkindHitcount += path.lines.length;
      }
      tupledCounts.set(`${pathkind}-${qkind}`, { hits: qkindHitcount, files: data[pathkind][qkind].length });
      pathkindHits += qkindHitcount;
      pathkindFiles += data[pathkind][qkind].length;
    }

    pathkindCounts.set(pathkind, { hits: pathkindHits, files: pathkindFiles });
  }

  // Accumulate the total number of files to support our "automatically go to a
  // single result" UX optimization below.
  var fileCount = 0;
  for (var pathkind in data) {
    if (pathkind.startsWith("*")) {
      continue;
    }
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
      // If there was only a "file" response just act like we clicked on the
      // file def by using a line number of 1.
      var lno = line?.lno || 1;
      window.location = `/${Dxr.tree}/source/${path}#${lno}`;
    } else {
      window.location = `/${Dxr.tree}/source/${path}`;
    }
    return;
  }

  // If no data is returned, inform the user.
  let container = document.getElementById("content");
  // Clobber any additional classes that existed on the content container.
  container.setAttribute("class", "content");

  const items = [];

  const breadcrumbs = document.querySelector(".breadcrumbs");
  if (breadcrumbs) {
    // Preserve breadcrumbs if present.
    items.push(breadcrumbs);

    // Breadcrumbs is hidden in some page.
    breadcrumbs.style.display = "inline-block";

    // Remove path and symbols.
    //
    // NOTE: Search can be initiated from source or directory listing,
    //       where breadcrumbs has path for the file or directory.
    let foundSep = false;
    for (const node of [...breadcrumbs.childNodes]) {
      if (node instanceof HTMLElement) {
        if (node.classList.contains("path-separator")) {
          foundSep = true;
        }
      }
      if (foundSep) {
        breadcrumbs.removeChild(node);
      }
    }
  }
  const navigationPanel = document.querySelector("#panel");
  if (navigationPanel) {
    // Preserve navigation panel if present.
    items.push(navigationPanel);

    try {
      Panel.prepareForSearch();
    } catch {}
  }

  if (!fileCount) {
    const div = document.createElement("div");
    div.textContent = "No results for current query.";
    items.push(div);
  } else {
    if (count) {
      const div = document.createElement("div");
      div.textContent = `Number of results: ${count} (maximum is around 4000)`;
      items.push(div);
    }
  }

  if (limits_hit.length > 0) {
    const div = document.createElement("div");
    const b = document.createElement("b");
    b.textContent = "Warning";
    div.append(b);
    div.append(`: The following limits were hit in your search: ${limits_hit.join(", ")}`);
    items.push(div);
  }

  let timeoutWarning = null;
  if (timed_out) {
    const div = document.createElement("div");
    const b = document.createElement("b");
    b.textContent = "Warning";
    div.append(b);
    div.append(": Results may be incomplete due to server-side search timeout!");
    items.push(div);
  }

  if (fileCount) {
    var table = document.createElement("table");
    table.className = "results";
    items.push(table);

    var counter = 0;

    var pathkindNames = {
      // Previously we would not say normal, but we need a place to hang the
      // counts.
      normal: "Core code",
      test: "Test files",
      generated: "Generated code",
      thirdparty: "Third-party code",
    };

    var html = "";
    // Loop over normal/test/generated/thirdparty "pathkind"s
    for (var pathkind in data) {
      if (pathkind.startsWith("*")) {
        continue;
      }
      var pathkindName = pathkindNames[pathkind];
      if (pathkindName) {
        let pathkindCount = pathkindCounts.get(pathkind);
        if (pathkindCount) {
          if (pathkindCount.hits) {
            maybeCounts = ` (${pathkindCount.hits} lines across ${pathkindCount.files} files)`;
          } else {
            maybeCounts = ` (${pathkindCount.files} files)`;
          }
        }

        html += "<tr><td>&nbsp;</td></tr>";
        html +=
          "<tr><td class='section'>§</td><td><div class='result-pathkind'>" +
          pathkindName + maybeCounts +
          "</div></td></tr>";
      }

      // Loop over definition/declaration/use/etc. "qkind"s
      for (var qkind in data[pathkind]) {
        if (data[pathkind][qkind].length) {
          html += "<tr><td>&nbsp;</td></tr>";

          html += "<tr><td class='left-column'>";
          html +=
            "<div class='expando open' data-klass='" +
            classOfResult(pathkind, qkind) +
            "'>&#9660;</div>";
          html += "</td>";

          let maybeCounts = "";
          let qkindCount = tupledCounts.get(`${pathkind}-${qkind}`);
          if (qkindCount) {
            if (qkindCount.hits) {
              maybeCounts = ` (${qkindCount.hits} lines across ${qkindCount.files} files)`;
            } else {
              maybeCounts = ` (${qkindCount.files} files)`;
            }
          }
          html += "<td><h2 class='result-kind'>" + escape(qkind) + maybeCounts + "</h2></td></tr>";
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

            let has_context = line.context_before || line.context_after;

            if (line.context_before) {
              let lineDelta = -line.context_before.length;
              for (const lineStr of line.context_before) {
                html += renderSingleSearchResult(
                  pathkind,
                  qkind,
                  file,
                  { lno: line.lno + lineDelta, line: lineStr },
                  "before",
                  true
                );
                lineDelta++;
              }
            }
            html += renderSingleSearchResult(pathkind, qkind, file, line, false, has_context);
            if (line.context_after) {
              let lineDelta = 1;
              for (const lineStr of line.context_after) {
                html += renderSingleSearchResult(
                  pathkind,
                  qkind,
                  file,
                  { lno: line.lno + lineDelta, line: lineStr },
                  "after",
                  true
                );
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

  container.replaceChildren(...items);

  Dxr.parseColQuery();
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
