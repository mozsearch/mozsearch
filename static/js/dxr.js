/**
 * Because we have a fixed header and often link to anchors inside pages, we can
 * run into the situation where the highled anchor is hidden behind the header.
 * This ensures that the highlighted anchor will always be in view.
 * @param {string} id = The id of the highlighted table row
 */
function scrollIntoView(id, navigate = true) {
  if (document.getElementById(id)) {
    return;
  }

  var firstLineno = id.split(/[,-]/)[0];
  var elt = document.getElementById("l" + firstLineno);

  var gotoElt = document.createElement("div");
  gotoElt.id = id;
  gotoElt.className = "goto";
  elt.appendChild(gotoElt);

  // Need this for Chrome.
  if (navigate && navigator.userAgent.indexOf("Firefox") == -1) {
    window.location = window.location;
  }
}

String.prototype.hashCode = function() {
  var hash = 0;
  if (this.length == 0) return hash;
  for (i = 0; i < this.length; i++) {
    char = this.charCodeAt(i);
    hash = ((hash<<5)-hash)+char;
    hash = hash & hash; // Convert to 32bit integer
  }
  return hash;
}

$(function() {
  'use strict';

  var constants = $('#data');
  var dxr = {},
  docElem = document.documentElement;

  dxr.wwwRoot = constants.data('root');
  dxr.baseUrl = location.protocol + '//' + location.host;
  dxr.icons = dxr.wwwRoot + '/static/icons/';
  dxr.views = dxr.wwwRoot + '/static/templates';
  dxr.searchUrl = constants.data('search');
  dxr.tree = constants.data('tree');

  var timeouts = {};
  timeouts.search = 300;
  // We start the history timeout after the search updates (i.e., after
  // timeouts.search has elapsed).
  timeouts.history = 2000 - timeouts.search;

  // Check if the currently loaded page has a hash in the URL
  if (window.location.hash) {
    scrollIntoView(window.location.hash.substr(1));
  }

  // We also need to cater for the above scenario when a user clicks on in page links.
  window.onhashchange = function() {
    scrollIntoView(window.location.hash.substr(1));
  };

  /**
   * Hang an advisory message off the search field.
   * @param {string} level - The seriousness: 'info', 'warning', or 'error'
   * @param {string} html - The HTML message to be displayed
   */
  function showBubble(level, html, which) {
    // If hideBubble() was already called, abort the hide animation:
    $(".bubble").stop();

    if (!which) {
      if ($("#path-bubble").is(":focus")) {
        which = "path";
      } else {
        which = "query";
      }
    }

    var other = which == "path" ? "query" : "path";

    $(`#${which}-bubble`).html(html)
      .removeClass("error warning info")
      .addClass(level)
      .show();

    $(`#${other}-bubble`).fadeOut(300);
  }

  function hideBubbles() {
    $(".bubble").fadeOut(300);
  }

  /**
   * If the `case` param is in the URL, returns its boolean value. Otherwise,
   * returns null.
   */
  function caseFromUrl() {
    if (window.location.pathname.endsWith("/search")) {
      var match = /[?&]case=([^&]+)/.exec(location.search);
      return match ? (match[1] === 'true') : false;
    } else {
      return null;
    }
  }

  var searchForm = $('#basic_search'),
  queryField = $('#query'),
  pathField = $('#path'),
  caseSensitiveBox = $('#case'),
  regexpBox = $('#regexp'),
  contentContainer = $('#content'),
  waiter = null,
  historyWaiter = null,
  nextRequestNumber = 1, // A monotonically increasing int that keeps old AJAX requests in flight from overwriting the results of newer ones, in case more than one is in flight simultaneously and they arrive out of order.
  requestsInFlight = 0,  // Number of search requests in flight, so we know whether to hide the activity indicator
  displayedRequestNumber = 0,
  resultCount = 0,
  dataOffset = 0,
  previousDataLimit = 0,
  defaultDataLimit = 100;

  window.addEventListener("pageshow", function() {
    function getQuery(key) {
      var val = new RegExp('[&?]' + key + '=([^&]*)').exec(location.search);
      if (val) {
        val = val[1];
        val = val.replace(/\+/g, ' ');
        val = decodeURIComponent(val);
        return val;
      }
    }
    var initialSearch = getQuery('q');
    if (initialSearch) {
      queryField.val(initialSearch);
    }

    var initialPath = getQuery('path');
    if (initialPath) {
      pathField.val(initialPath);
    }

    var regexp = getQuery('regexp') === 'true';
    regexpBox.prop('checked', regexp);
  });

  /**
   * Returns the full Ajax URL for search and explicitly sets
   * redirect to false and format to json to ensure we never
   * get a HTML response or redirect from an Ajax call, even
   * when using the back button.
   *
   * @param {string} query - The query string
   * @param {bool} isCaseSensitive - Whether the query should be case-sensitive
   */
  function buildAjaxURL(query) {
    var search = dxr.searchUrl;
    var params = {};
    params.q = query;
    params['case'] = caseSensitiveBox.prop('checked');
    params.regexp = regexpBox.prop('checked');
    params.path = $.trim(pathField.val());

    return search + '?' + $.param(params);
  }

  /**
   * Add an entry into the history stack whenever we do a new search.
   */
  function pushHistoryState(searchUrl) {
    history.pushState({}, '', searchUrl);
  }

  /**
   * Clears any existing query timer and sets a new one to query in a moment.
   */
  function querySoon() {
    clearTimeout(waiter);
    clearTimeout(historyWaiter);
    waiter = setTimeout(doQuery, timeouts.search);
  }

  /**
   * Saves checkbox checked property to localStorage and invokes queryNow function.
   */
  function updateLocalStorageAndQueryNow(){
    localStorage.setItem('caseSensitive', $('#case').prop('checked'));
    queryNow();
  }

  function classOfResult(pathkind, qkind) {
    var klass = pathkind + ":" + qkind;
    klass = String(klass.hashCode());
    return "EXPANDO" + klass;
  }

  function onExpandoClick(event) {
    var target = $(event.target);
    var open = target.hasClass("open");

    if (open) {
      $("." + target.data("klass")).hide();
      target.removeClass("open");
      target.html("+");
    } else {
      $("." + target.data("klass")).show();
      target.addClass("open");
      target.html("&#8722;");
    }
  }

  /**
   * Clears any existing query timer and queries immediately.
   */
  function queryNow() {
    clearTimeout(waiter);
    doQuery();
  }

  var populateEpoch = 0;

  function populateResults(data, full, jumpToSingle) {
    populateEpoch++;

    var title = data["*title*"];
    if (title) {
      delete data["*title*"];
      document.title = title + " - mozsearch";
    }

    window.scrollTo(0, 0);

    function makeURL(path) {
      return "/" + dxr.tree + "/source/" + path;
    }

    function chooseIcon(path) {
      var suffix = path.lastIndexOf(".");
      if (suffix == -1) {
        return "unknown";
      }
      suffix = path.slice(suffix + 1);
      return {
        'cpp': 'cpp',
        'h': 'h',
        'c': 'c',
        'mm': 'mm',
        'js': 'js',
        'jsm': 'js',
        'py': 'py',
        'ini': 'conf',
        'sh': 'sh',
        'txt': 'txt',
        'xml': 'xml',
        'xul': 'ui',
        'java': 'java',
        'in': 'txt',
        'html': 'html',
        'png': 'image',
        'gif': 'image',
        'svg': 'svg',
        'build': 'build',
        'json': 'js',
        'css': 'css',
      }[suffix] || "unknown";
    }

    function renderPath(pathkind, qkind, fileResult) {
      var klass = classOfResult(pathkind, qkind);

      var html = "";
      html += "<tr class='result-head " + klass + "'>";
      html += "<td class='left-column'><div class='" + chooseIcon(fileResult.path) + " icon-container'></div></td>";

      html += "<td>";

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

      html += "</td>";
      html += "</tr>"

      return html;
    }

    function renderSingleSearchResult(pathkind, qkind, file, line) {
      var [start, end] = line.bounds || [0, 0];
      var before = line.line.slice(0, start).replace(/^\s+/, "");
      var middle = line.line.slice(start, end);
      var after = line.line.slice(end).replace(/\s+$/, "");

      var klass = classOfResult(pathkind, qkind);
      var html = "";
      html += "<tr class='" + klass + "'>";
      html += "<td class='left-column'><a href='" + makeURL(file.path) + "#" + line.lno + "'>" +
        line.lno + "</a></td>";
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
          var url = `/${dxr.tree}/search?q=symbol:${encodeURIComponent(line.contextsym)}&redirect=false`;
          inside = "<a href='" + url + "'>" + line.context + "</a>";
        }
        html += " <span class='result-context'>// found in <code>" + inside + "</code></span>";
      }

      html += "</td>";
      html += "</tr>";

      return html;
    }

    var count = 0;
    for (var pathkind in data) {
      for (var qkind in data[pathkind]) {
        for (var k = 0; k < data[pathkind][qkind].length; k++) {
          var path = data[pathkind][qkind][k];
          count += path.lines.length;
        }
      }
    }

    var fileCount = 0;
    for (var pathkind in data) {
      for (var qkind in data[pathkind]) {
        fileCount += data[pathkind][qkind].length;
      }
    }

    if (jumpToSingle && fileCount == 1 && count <= 1) {
      var pathkind = Object.keys(data)[0];
      var qkind = Object.keys(data[pathkind])[0];
      var file = data[pathkind][qkind][0];
      var path = file.path;

      if (count == 1) {
        var line = file.lines[0];
        var lno = line.lno;
        window.location = `/${dxr.tree}/source/${path}#${lno}`;
      } else {
        window.location = `/${dxr.tree}/source/${path}`;
      }
      return;
    }

    // If no data is returned, inform the user.
    if (!fileCount) {
      var user_message = contentContainer.data('no-results');
      contentContainer.empty().append($("<span>" + user_message + "</span>"));
    } else {
      var container = contentContainer.empty();

      if (count) {
        var numResults = $(`<div>Number of results: ${count} (maximum is 1000)</div>`);
        container.append(numResults);
      }

      var table = document.createElement("table");
      table.className = "results";

      container.append($(table));

      var counter = 0;

      var pathkindNames = {
        "normal": null,
        "test": "Test files",
        "generated": "Generated code",
      };

      var html = "";
      for (var pathkind in data) {
        var pathkindName = pathkindNames[pathkind];
        if (pathkindName) {
          html += "<tr><td>&nbsp;</td></tr>"
          html += "<tr><td class='section'>§</td><td><div class='result-pathkind'>" + pathkindName + "</div></td></tr>"
        }

        var qkinds = Object.keys(data[pathkind]);
        for (var qkind in data[pathkind]) {
          if (data[pathkind][qkind].length) {
            html += "<tr><td>&nbsp;</td></tr>";

            html += "<tr><td class='left-column'>";
            html += "<div class='expando open' data-klass='" + classOfResult(pathkind, qkind) + "'>&#8722;</div>";
            html += "</td>";

            html += "<td><div class='result-kind'>" + qkind + "</div></td></tr>";
          }

          for (var i = 0; i < data[pathkind][qkind].length; i++) {
            var file = data[pathkind][qkind][i];

            if (counter > 100 && !full) {
              break;
            }

            html += renderPath(pathkind, qkind, file);

            file.lines.map(function(line) {
              counter++;
              if (counter > 100 && !full) {
                return;
              }

              html += renderSingleSearchResult(pathkind, qkind, file, line);
            });
          }
        }
      }

      table.innerHTML = html;

      $(".expando").click(onExpandoClick);

      if (counter > 100 && !full) {
        var epoch = populateEpoch;
        setTimeout(function() {
          if (populateEpoch == epoch) {
            populateResults(data, true, false);
          }
        }, 750);
      }
    }
  }

  window.showSearchResults = function(results) {
    var jumpToSingle = window.location.search.indexOf("&redirect=false") == -1;
    populateResults(results, true, jumpToSingle);
  };

  /**
   * Queries and populates the results templates with the returned data.
   */
  function doQuery() {
    function oneMoreRequest() {
      if (requestsInFlight === 0) {
        $('#search-box').addClass('in-progress');
      }
      requestsInFlight += 1;
    }

    function oneFewerRequest() {
      requestsInFlight -= 1;
      if (requestsInFlight === 0) {
        $('#search-box').removeClass('in-progress');
      }
    }

    clearTimeout(historyWaiter);

    var query = $.trim(queryField.val());
    var pathFilter = $.trim(pathField.val());

    var myRequestNumber = nextRequestNumber;

    if (query.length == 0 && pathFilter.length == 0) {
      hideBubbles();
      return;
    }

    if (query.length < 3 && pathFilter.length < 3) {
      showBubble("info", "Enter at least 3 characters to do a search.",
                 query.length ? "query" : "path");
      return;
    }

    hideBubbles();

    nextRequestNumber += 1;
    oneMoreRequest();
    var searchUrl = buildAjaxURL(query);
    $.getJSON(searchUrl, function(data) {
      // New results, overwrite
      if (myRequestNumber > displayedRequestNumber) {
        displayedRequestNumber = myRequestNumber;
        populateResults(data, false, false);
        historyWaiter = setTimeout(pushHistoryState, timeouts.history, searchUrl);
      }
      oneFewerRequest();
    })
      .fail(function(jqxhr) {
        oneFewerRequest();

        // A newer response already arrived and is displayed. Don't bother complaining about this old one.
        if (myRequestNumber < displayedRequestNumber)
          return;

        if (jqxhr.responseJSON)
          showBubble(jqxhr.responseJSON.error_level, jqxhr.responseJSON.error_html);
        else
          showBubble("error", "An error occurred. Please try again.");
      });
  }

  // Do a search every time you pause typing for 300ms:
  queryField.on('input', querySoon);
  pathField.on('input', querySoon);

  // Update the search when the case-sensitive box is toggled, canceling any pending query:
  caseSensitiveBox.on('change', updateLocalStorageAndQueryNow);

  regexpBox.on('change', queryNow);


  var urlCaseSensitive = caseFromUrl();
  if (urlCaseSensitive !== null) {
    // Any case-sensitivity specification in the URL overrides what was in localStorage:
    localStorage.setItem('caseSensitive', urlCaseSensitive);
    caseSensitiveBox.prop('checked', urlCaseSensitive);
  } else {
    // Restore checkbox state from localStorage:
    caseSensitiveBox.prop('checked', 'true' === localStorage.getItem('caseSensitive'));
  }


  /**
   * Adds a leading 0 to numbers less than 10 and greater that 0
   *
   * @param int number The number to test against
   *
   * return Either the original number or the number prefixed with 0
   */
  function addLeadingZero(number) {
    return (number <= 9) || (number === 0) ? "0" + number : number;
  }

  /**
   * Converts string to new Date and returns a formatted date in the
   * format YYYY-MM-DD h:m
   * @param String dateString A date in string form.
   *
   */
  function formatDate(dateString) {
    var fullDateTime = new Date(dateString),
    date = fullDateTime.getFullYear() + '-' + (fullDateTime.getMonth() + 1) + '-' + addLeadingZero(fullDateTime.getDate()),
    time = fullDateTime.getHours() + ':' + addLeadingZero(fullDateTime.getMinutes());

    return date + ' ' + time;
  }

  var prettyDate = $('.pretty-date');
  prettyDate.each(function() {
    $(this).text(formatDate($(this).data('datetime')));
  });

  // Thanks to bug 63040 in Chrome, onpopstate is fired when the page reloads.
  // That means that if we naively set onpopstate, we would get into an
  // infinite loop of reloading whenever onpopstate is triggered. Therefore,
  // we have to only add our onpopstate handler once the page has loaded.
  window.onload = function() {
    setTimeout(function() {
      window.onpopstate = popStateHandler;
    }, 0);
  };

  // Reload the page when we go back or forward.
  function popStateHandler(event) {
    // FIXME: This reloads the page when you navigate to #lineno.
    window.onpopstate = null;
    window.location.reload();
  }

  /**
   * Replace 'source' with 'raw' in href, and set that to the background-image
   */
  function setBackgroundImageFromLink(anchorElement) {
    var href = anchorElement.getAttribute('href');
    // note: breaks if the tree's name is "source"
    var bg_src = href.replace('source', 'raw');
    anchorElement.style.backgroundImage = 'url(' + bg_src + ')';
  }

  window.addEventListener('load', function() {
    $(".image").not('.too_fat').each(function() {
      setBackgroundImageFromLink(this);
    });
  });

});
