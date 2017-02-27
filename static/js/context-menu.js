/**
 * Dynamically updating 2-tier menu, allowing for the menu to be immediately
 * displayed with what's available and to update as search results arrive.
 * Replaces the nunjucks-based template mechanism.  (nunjucks supports async
 * rendering that delays the render, but does not seem to support streaming
 * DOM updates.)
 *
 * Menu items must have:
 * - label: The textContent of the <li> menu item; no HTML.
 * - href: The link to apply to the label.
 * - icon: Class name of the icon to display for the menu item.
 *
 * They may also have:
 * - populateSubmenu: A function(elem) that takes a <div> element that should
 *   be populated at some point.  If you have the info already, populate
 *   synchronously.  If you are doing something asynchronously, populate it when
 *   you have the data, keeping in mind that the element may no longer be in the
 *   document by the time you go to populate the element.
 */
function MegaMenu(menuDef) {
  var rootElem = this.rootElem = document.createElement('ul');
  rootElem.id = 'context-menu';
  rootElem.className = 'context-menu';
  rootElem.setAttribute('tabindex', '0');

  this.itemCount = 0;
  this.addMenuItems(menuDef.menuItems);
}
MegaMenu.prototype = {
  addMenuItem: function(item) {
    var listElem = document.createElement('li');
    var linkElem = document.createElement('a');
    linkElem.href = item.href;
    linkElem.className = item.icon + ' icon context-menu-item';
    linkElem.textContent = item.label;

    var itemId = 'submenu-' + this.itemCount++;
    linkElem.id = itemId;
    listElem.appendChild(linkElem);

    if (item.populateSubmenu) {
      var subElem = document.createElement('div');
      subElem.className = 'context-submenu';
      try {
        item.populateSubmenu(subElem);
        listElem.appendChild(subElem);
      } catch(ex) {
        console.warn('Problem populating submenu:', ex);
      }
    }

    this.rootElem.appendChild(listElem);
  },

  addMenuItems: function(items) {
    for (var i = 0; i < items.length; i++) {
      this.addMenuItem(items[i]);
    }
  },
};

function setContextMenu(menu, event)
{
  var selObj = window.getSelection();
  if (selObj.toString() != "") {
    // Don't display the context menu if there's a selection.
    // User could be trying to select something and the context menu will undo it.
    return;
  }

  var top = event.clientY + window.scrollY;
  var left = event.clientX;

  var megaMenu = new MegaMenu(menu);
  $('body').append(megaMenu.rootElem);
  var currentContextMenu = $('#context-menu');
  currentContextMenu.css({
    top: top - 4,
    left: left - 4
  });

  // Move focus to the context menu
  //currentContextMenu[0].focus();

  currentContextMenu.menuAim({
    submenuSelector: '.context-menu-item',
    activate: function(row) {
      var subElem = row.querySelector('.context-submenu');

      $(subElem).css({
        display: 'block',
        top: -1,
        left: currentContextMenu.width(),
        minHeight: currentContextMenu.outerHeight()
      });

      row.querySelector('a.context-menu-item').classList.add('context-menu-maintain-hover');
    },
    deactivate: function(row) {
      var subElem = row.querySelector('.context-submenu');
      if (subElem) {
        subElem.style.display = 'none';
      }
      row.querySelector('a.context-menu-item').classList.remove('context-menu-maintain-hover');
    }
  });

  currentContextMenu.on('mousedown', function(event) {
    // Prevent clicks on the menu to propagate
    // to the window, so that the menu is not
    // removed and links will be followed.
    event.stopPropagation();
  });
}

// Remove the menu when a user clicks outside it.
window.addEventListener('mousedown', function() {
  $('#context-menu').remove();
}, false);

window.addEventListener("pageshow", function() {
  $('#context-menu').remove();
}, false);

var hovered = $();

$("#file").on("mousemove", function(event) {
  if ($('#context-menu').length) {
    return;
  }

  var y = event.clientY;
  var x = event.clientX;

  var elt = document.elementFromPoint(x, y);
  while (!elt.hasAttribute("data-id")) {
    elt = elt.parentNode;
    if (!elt || !(elt instanceof Element)) {
      hovered.removeClass("hovered");
      hovered = $();
      return;
    }
  }

  elt = $(elt);
  var id = elt.attr("data-id");

  if (id == "?") {
    hovered.removeClass("hovered");
    hovered = $();
    return;
  }

  hovered.removeClass("hovered");
  hovered = $(`span[data-id="${id}"]`);
  hovered.addClass("hovered");

  var index = elt.attr("data-i");
  if (index) {
    var [jumps, searches] = ANALYSIS_DATA[index];
    if (searches.length) {
      loadSymbolInfo(searches[0].sym);
    }
  }
});

/**
 * Cache from symbol-name to promise that gets resolved
 */
var CACHED_SYMBOL_SEARCHES = new Map();
function loadSymbolInfo(sym) {
  if (CACHED_SYMBOL_SEARCHES.has(sym)) {
    return CACHED_SYMBOL_SEARCHES.get(sym);
  }
  var promise = new Promise(function(resolve, reject) {
    var searchUrl = buildAjaxURL('symbol:' + sym);
    $.getJSON(searchUrl, function(data) {
      resolve(normalizeSymbolInfo(data));
    }).fail(function() { reject(); });
  });
  CACHED_SYMBOL_SEARCHES.set(sym, promise);
  return promise;
}
/**
 * Reprocess the symbol search results slightly.  Specifically:
 * - Only keep "normal" results.
 * - Re-partition and re-sort into 2 buckets, discarding the pretty key
 *   formatting:
 *   - declDefs: Declarations then definitions, in that order.
 *   - uses: Still uses.
 */
function normalizeSymbolInfo(data) {
  var decls = [];
  var defs = [];
  var uses = [];
  // Count up the total number of uses over all files/lines.  uses.length only
  // represents the number of files containing uses.
  var totalUses = 0;

  // Filter predicate that drops forward declarations.
  // XXX It seems like searchfox should probably move forward declarations to
  // a different kind?
  function dropForwardDecls(fileResult) {
    if (fileResult.lines.length === 1) {
      var lineContent = fileResult.lines[0].line;
      // Our goal is to drop declaration lines that look like:
      // "struct Foo;" "class Foo;", in particular without an opening brace or
      // delimiting whitespace.
      return !(/^(?:class|struct) [^{\s]+;$/.test(lineContent));
    }
    return true;
  }

  function chewQKinds(qkinds) {
    for (var qk in qkinds) {
      var value = qkinds[qk];
      var prettySymbol;

      // Discriminate based on the 3rd letter which varies over all types.
      switch (qk[2]) {
        case "c": // "Declarations (".length === 14
          prettySymbol = qk.slice(14, -1);
          decls = decls.concat(value.filter(dropForwardDecls));
          break;
        case "f": // "Definitions (".length === 13
          prettySymbol = qk.slice(13, -1);
          defs = defs.concat(value);
          break;
        case "e": // "Uses (".length === 6
          uses = uses.concat(value);
          totalUses += value.reduce(function(accumulated, fileResult) {
            return accumulated + fileResult.lines.length;
          }, 0);
          break;
        case "s": // "Assignments (".length === 13
          // Not used yet.
          continue;
        case "L": // "IDL (".length === 5
          prettySymbol = qk.slice(5, -1);
          decls = decls.concat(value);
          break;
      }
      // It might be appropriate assert the pretty symbol matches here.
    }
  }

  chewQKinds(data.normal || []);

  return {
    declDefs: decls.concat(defs),
    uses: uses,
    totalUses: totalUses
  };
}

function makeSourceURL(path) {
  return "/" + dxr.tree + "/source/" + path;
}

function makePathElements(path, className, optionalLno) {
  var containerElem = document.createElement('div');
  containerElem.className = className;

  var pathParts = path.split('/');
  var pathSoFar = '';
  for (var i = 0; i < pathParts.length; i++) {
    if (i != 0) {
      var sepElem = document.createElement('span');
      sepElem.className = 'path-separator';
      sepElem.textContent = '/';
      containerElem.appendChild(sepElem);
    }

    var part = pathParts[i];
    pathSoFar += part;

    var linkElem = document.createElement('a');
    linkElem.href = makeSourceURL(pathSoFar);
    linkElem.textContent = part;
    containerElem.appendChild(linkElem);

    pathSoFar += '/';
  }

  if (optionalLno) {
    var lineElem = document.createElement('a');
    lineElem.href = makeSourceURL(path + '#' + optionalLno);
    lineElem.textContent = ' #' + optionalLno;
    containerElem.appendChild(lineElem);
  }

  return containerElem;
}

/**
 * Populate the submenu with the decl/defs for the given symbol, favoring the
 * full comment if available, failing over to the single "line" excerpt
 * otherwise.
 */
function makeCommentSubmenuPopulater(sym) {
  return function(menuElem) {
    loadSymbolInfo(sym).then(function(info) {
      info.declDefs.forEach(function(fileResult) {
        var path = fileResult.path;
        var lines = fileResult.lines;
        // In the single hit case (which should be all of them for decls/defs),
        // display the line number in the header portion.
        var useLno = null;
        if (lines.length === 1) {
          useLno = lines[0].lno;
        }

        var container = document.createElement('div');
        container.appendChild(
          makePathElements(path, 'submenu-path-header', useLno));

        for (var iLine = 0; iLine < lines.length; iLine++) {
          var line = lines[iLine];

          var linkElem = document.createElement('a');
          linkElem.href = makeSourceURL(path + '#' + line.lno);

          var codeElem = document.createElement('pre');
          if (line.rawComment) {
            codeElem.textContent = line.rawComment;
          } else {
            codeElem.textContent = line.line;
          }

          linkElem.appendChild(codeElem);
          container.appendChild(linkElem);
        }

        menuElem.appendChild(container);
      });
    });
  };
}

/**
 * Populate the uses submenu.  We have 2 modes of operation:
 * - Small result count, show everything.
 * - Many results, hierarchically group, showing tallies.
 */
function makeUsesSubmenuPopulater(sym) {
  return function(menuElem) {
    loadSymbolInfo(sym).then(function(info) {
      if (info.totalUses < 6) {
        // Small result count, show everything.
        info.uses.forEach(function(fileResult) {
          var path = fileResult.path;
          var lines = fileResult.lines;
          // In the single hit case (which should be all of them for decls/defs),
          // display the line number in the header portion.
          var useLno = null;
          if (lines.length === 1) {
            useLno = lines[0].lno;
          }

          var container = document.createElement('div');
          container.appendChild(
            makePathElements(path, 'submenu-path-header', useLno));

          for (var iLine = 0; iLine < lines.length; iLine++) {
            var line = lines[iLine];

            if (line.context) {
              var contextElem = document.createElement('div');
              contextElem.className = 'submenu-context-header';
              contextElem.textContent = line.context;
              container.appendChild(contextElem);
            }

            var linkElem = document.createElement('a');
            linkElem.href = makeSourceURL(path + '#' + line.lno);

            var codeElem = document.createElement('pre');
            codeElem.textContent = line.line;

            linkElem.appendChild(codeElem);
            container.appendChild(linkElem);
          }

          menuElem.appendChild(container);
        });
      } else {
        // Overload mode.
        // TODO: Cluster.
        menuElem.textContent = `There's a whopping ${info.totalUses} uses!`;
      }
    });
  }
}


$("#file").on("click", "span[data-i]", function(event) {
  var tree = $("#data").data("tree");

  var elt = $(event.target);
  while (!elt.attr("data-i")) {
    elt = elt.parent();
  }
  var index = elt.attr("data-i");

  function fmt(s, data) {
    return s.replace("_", data);
  }

  // Comes from the generated page.
  var [jumps, searches] = ANALYSIS_DATA[index];

  var menuItems = [];

  // HACK: ANALYSIS_DATA currently has no idea that we like to expose the
  // decl/defs in our mega-menu, so create a synthetic menu item with a
  // different label so that we can expose the information even when the user
  // is clicking on the canonical definition token.
  var syntheticJump = false;
  if (!jumps.length && searches.length === 1) {
    syntheticJump = true;
    jumps.push(searches[0]);
  }
  for (var i = 0; i < jumps.length; i++) {
    var sym = jumps[i].sym;
    var pretty = jumps[i].pretty;
    var label;
    if (syntheticJump) {
      label = fmt("Declarations of _", pretty);
    } else {
      label = fmt("Go to definition of _", pretty);
    }
    menuItems.push({
      label: label,
      href: `/${tree}/define?q=${encodeURIComponent(sym)}&redirect=false`,
      icon: "search",
      populateSubmenu: makeCommentSubmenuPopulater(sym),
    });
  }

  for (var i = 0; i < searches.length; i++) {
    var sym = searches[i].sym;
    var pretty = searches[i].pretty;
    menuItems.push({
      label: fmt("Search for _", pretty),
      href: `/${tree}/search?q=symbol:${encodeURIComponent(sym)}&redirect=false`,
      icon: "search",
      populateSubmenu: makeUsesSubmenuPopulater(sym),
    });
  }

  setContextMenu({menuItems: menuItems}, event);
});
