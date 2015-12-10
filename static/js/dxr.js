/* jshint devel:true, esnext: true */
/* globals nunjucks: true, $ */

$(function() {
  'use strict';

  var constants = $('#data');
  var stateConstants = $('#state');
  var dxr = {},
  docElem = document.documentElement;

  dxr.wwwRoot = constants.data('root');
  dxr.baseUrl = location.protocol + '//' + location.host;
  dxr.icons = dxr.wwwRoot + '/static/icons/';
  dxr.views = dxr.wwwRoot + '/static/templates';
  dxr.searchUrl = constants.data('search');
  dxr.tree = constants.data('tree');

  var timeouts = {};
  timeouts.scroll = 500;
  timeouts.search = 300;
  // We start the history timeout after the search updates (i.e., after
  // timeouts.search has elapsed).
  timeouts.history = 2000 - timeouts.search;

  // Return the maximum number of pixels the document can be scrolled.
  function getMaxScrollY() {
    // window.scrollMaxY is a non standard implementation in
    // Gecko(Firefox) based browsers. If this is thus available,
    // simply return it, else return the calculated value above.
    // @see https://developer.mozilla.org/en-US/docs/Web/API/Window.scrollMaxY
    return window.scrollMaxY || (docElem.scrollHeight - window.innerHeight);
  }

  /**
   * Because we have a fixed header and often link to anchors inside pages, we can
   * run into the situation where the highled anchor is hidden behind the header.
   * This ensures that the highlighted anchor will always be in view.
   * @param {string} id = The id of the highlighted table row
   */
  function scrollIntoView(id) {
    var lineElement = document.getElementById(id);

    if (lineElement === null)  // There is no line #1. Empty file.
      return;

    if ((getMaxScrollY() - lineElement.offsetTop) > 100) {
      window.scroll(0, window.scrollY - 150);
    }
  }

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
  function showBubble(level, html) {
    // If hideBubble() was already called, abort the hide animation:
    $('.bubble').stop();

    $('.bubble').html(html)
      .removeClass('error warning info')
      .addClass(level)
      .show();
  }

  function hideBubble() {
    $('.bubble').fadeOut(300);
  }

  /**
   * If the `case` param is in the URL, returns its boolean value. Otherwise,
   * returns null.
   */
  function caseFromUrl() {
    var match = /[?&]?case=([^&]+)/.exec(location.search);
    return match ? (match[1] === 'true') : null;
  }

  var searchForm = $('#basic_search'),
  queryField = $('#query'),
  query = null,
  caseSensitiveBox = $('#case'),
  contentContainer = $('#content'),
  waiter = null,
  historyWaiter = null,
  nextRequestNumber = 1, // A monotonically increasing int that keeps old AJAX requests in flight from overwriting the results of newer ones, in case more than one is in flight simultaneously and they arrive out of order.
  requestsInFlight = 0,  // Number of search requests in flight, so we know whether to hide the activity indicator
  displayedRequestNumber = 0,
  didScroll = false,
  resultCount = 0,
  dataOffset = 0,
  previousDataLimit = 0,
  defaultDataLimit = 100;

  // Has the user been redirected to a direct result?
  var fromQuery = /[?&]?from=([^&]+)/.exec(location.search);
  if (fromQuery !== null) {
    // Offer the user the option to see all the results instead.
    var viewResultsTxt = 'Showing a direct result. <a href="{{ url }}">Show all results instead.</a>',
    isCaseSensitive = caseFromUrl();

    var searchUrl = constants.data('search') + '?q=' + fromQuery[1];
    if (isCaseSensitive !== null) {
      searchUrl += '&case=' + isCaseSensitive;
    }

    $('#query').val(decodeURIComponent(fromQuery[1]));
    showBubble('info', viewResultsTxt.replace('{{ url }}', searchUrl));
  }

  $(window).scroll(function() {
    didScroll = true;
  });

  /**
   * Returns the full Ajax URL for search and explicitly sets
   * redirect to false and format to json to ensure we never
   * get a HTML response or redirect from an Ajax call, even
   * when using the back button.
   *
   * @param {string} query - The query string
   * @param {bool} isCaseSensitive - Whether the query should be case-sensitive
   * @param {int} limit - The number of results to return.
   * @param [int] offset - The cursor position
   */
  function buildAjaxURL(query, isCaseSensitive, limit, offset) {
    var search = dxr.searchUrl;
    var params = {};
    params.q = query;
    params.redirect = false;
    params['case'] = isCaseSensitive;
    params.limit = limit;
    params.offset = offset;

    return search + '?' + $.param(params);
  }

  /**
   * Updates the window's history entry to not break the back button with
   * infinite scroll.
   * @param {int} offset - The offset to store in the URL
   */
  function setHistoryState(offset) {
    var state = {},
    re = /offset=\d+/,
    locationSearch = '';

    if (location.search.indexOf('offset') > -1) {
      locationSearch = location.search.replace(re, 'offset=' + offset);
    } else {
      locationSearch = location.search ? location.search + '&offset=' + offset : '?offset=' + offset;
    }

    var url = dxr.baseUrl + location.pathname + locationSearch + location.hash;

    history.replaceState(state, '', url);
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

  /**
   * Clears any existing query timer and queries immediately.
   */
  function queryNow() {
    clearTimeout(waiter);
    doQuery();
  }

  /**
   * Populates the results template.
   * @param {object} tmpl - The template to use to render results.
   * @param {object} data - The data returned from the query
   * @param {bool} append - Should the content be appended or overwrite
   */
  function populateResults(data, append) {
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

    function renderPath(fileResult) {
      var row = $("<tr class='result-head'></tr>");
      var icon = $("<td class='left-column'><div class='" + chooseIcon(fileResult.path) + " icon-container'></div></td>");
      row.append(icon);

      var main = $("<td></td>");
      row.append(main);

      var elts = fileResult.path.split("/");
      var pathSoFar = "";
      for (var i = 0; i < elts.length; i++) {
        if (i != 0) {
          main.append($("<span class='path-separator'>/</span>"));
        }

        var elt = elts[i];
        pathSoFar += "/" + elt;
        main.append($("<a href='" + makeURL(pathSoFar) + "'>" + elt + "</a>"));
      }

      return row;
    }

    function renderSingleSearchResult(file, line) {
      var row = $("<tr></tr>");
      row.append($("<td class='left-column'><a href='" + makeURL(file.path) + "#" + line.lno + "'>" +
                   line.lno + "</a></td>"));
      row.append($("<td><a href='" + makeURL(file.path) + "#" + line.lno + "'><code></code></a></td>"));
      $("code", row).text(line.line);
      return row;
    }

    var count = 0;
    for (var kind in data) {
      for (var k = 0; k < data[kind].length; k++) {
        var path = data[kind][k];
        count += path.lines.length;
      }
    }

    var fileCount = 0;
    for (var kind in data) {
      fileCount += data[kind].length;
    }

    var keyOrder = ["Definitions", "Assignments", "Uses", "default"];

    // If no data is returned, inform the user.
    if (!fileCount) {
      var user_message = contentContainer.data('no-results');
      contentContainer.empty().append($("<span>" + user_message + "</span>"));
    } else {
      var container = append ? contentContainer : contentContainer.empty();

      if (count) {
        var numResults = $(`<div>Number of results: ${count} (maximum is 1000)</div>`);
        container.append(numResults);
      }

      var table = $("<table class='results'></table>");
      container.append(table);

      for (var k = 0; k < keyOrder.length; k++) {
        var kind = keyOrder[k];
        if (!(kind in data)) {
          continue;
        }
        if (kind != "default" && data[kind].length) {
          var headerRow = $("<tr> <td class='left-column'></td><td><b>" + kind + "</b></td> </tr>");
          table.append(headerRow);
        }

        for (var i = 0; i < data[kind].length; i++) {
          var file = data[kind][i];
          var fileRow = renderPath(file);
          table.append(fileRow);

          var lineResults = file.lines.map(function(line) {
            return renderSingleSearchResult(file, line);
          });
          table.append(lineResults);
        }
      }
    }

    if (!append) {
      //document.title = data.query + " - mozsearch";
    }
  }

  window.showSearchResults = function(results) {
    populateResults(results, false);
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

    query = $.trim(queryField.val());
    var myRequestNumber = nextRequestNumber,
    lineHeight = parseInt(contentContainer.css('line-height'), 10),
    limit = previousDataLimit = parseInt((window.innerHeight / lineHeight) + 25);

    if (query.length === 0) {
      hideBubble();  // Don't complain when I delete what I typed. You didn't complain when it was empty before I typed anything.
      return;
    } else if (query.length < 3) {
      showBubble('info', 'Enter at least 3 characters to do a search.');
      return;
    }

    hideBubble();
    nextRequestNumber += 1;
    oneMoreRequest();
    var searchUrl = buildAjaxURL(query, caseSensitiveBox.prop('checked'), limit);
    $.getJSON(searchUrl, function(data) {
      // New results, overwrite
      if (myRequestNumber > displayedRequestNumber) {
        displayedRequestNumber = myRequestNumber;
        populateResults(data, false);
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
          showBubble('error', 'An error occurred. Please try again.');
      });
  }

  // Do a search every time you pause typing for 300ms:
  queryField.on('input', querySoon);

  // Update the search when the case-sensitive box is toggled, canceling any pending query:
  caseSensitiveBox.on('change', updateLocalStorageAndQueryNow);


  var urlCaseSensitive = caseFromUrl();
  if (urlCaseSensitive !== null) {
    // Any case-sensitivity specification in the URL overrides what was in localStorage:
    localStorage.setItem('caseSensitive', urlCaseSensitive);
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
    // Check for event state first to avoid nasty complete page reloads on #anchors:
    if (event.state != null) {
      window.onpopstate = null;
      window.location.reload();
    }
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
