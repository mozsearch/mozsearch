/**
 * Dynamically updating 2-tier menu w/header, allowing for the menu to be
 * immediately displayed with what's available and to update as search results
 * arrive.
 *
 * The menuDef consists of:
 * - header: { label, href }.  A header that spans the primary menu column and
 *   the secondary detail popup that they trigger.
 * - menuItems: See below.
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
 *
 * ## Layout
 *
 * CSS Grid would be ideal for this, but it's not available yet.  So we're
 * adopting a flexbox layout.  See the block comment above ".context-menu" in
 * mozsearch.css for some idea of what's going on.
 */
function MegaMenu(menuDef) {
  var rootElem = this.rootElem = document.createElement('div');
  rootElem.id = 'context-menu';
  rootElem.className = 'context-menu';
  rootElem.setAttribute('tabindex', '0');

  if (menuDef.header) {
    var headerElem = this.headerElem = document.createElement('h3');
    headerElem.className = 'context-menu-header';
    headerElem.textContent = menuDef.header.label;
    rootElem.appendChild(headerElem);
  }

  var menuBody = document.createElement('div');
  menuBody.className = 'context-menu-body';
  rootElem.appendChild(menuBody);

  var menuItemContainer = this.menuItemContainer = document.createElement('ul');
  menuItemContainer.className = 'context-menu-item-container';
  menuBody.appendChild(menuItemContainer);

  var submenuContainer = this.submenuContainer = document.createElement('div');
  submenuContainer.className = 'context-submenu-container';
  menuBody.appendChild(submenuContainer);

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
        item.populateSubmenu(subElem, this);
        linkElem.subMenuElem = subElem; // save it on an expando.
        this.submenuContainer.appendChild(subElem);
      } catch(ex) {
        console.warn('Problem populating submenu:', ex);
      }
    }

    this.menuItemContainer.appendChild(listElem);
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

  var menuWidth = megaMenu.rootElem.offsetWidth;
  var viewportWidth = window.innerWidth;
  var obscuredMenuWidth = Math.max((left + menuWidth) - viewportWidth, 0);
  var itemWidth = megaMenu.menuItemContainer.offsetWidth;
  // Positioning goals:
  // * Have the first menu item be beneath the user's mouse so its submenu is
  //   visible.
  // * Have the menu be horizontally visible on the screen.
  // * Try and avoid having the user's mouse obscure the menu options.  (Icons
  //   help with this.)
  //
  // Our main adjustment is then to conceptually slide the menu left until the
  // menu is horizontally on the screen or the mouse would no longer be over the
  // first menu item.
  currentContextMenu.css({
    top: top - megaMenu.headerElem.offsetHeight - 8,
    // In my non-extensive testing, itemWidth right now ends up being sized at
    // ~17 pixels wider than it actually ends up and I'm not really quite sure
    // why that is.  I'm subtracting that off and a little extra as a hack for
    // now.  Once the menu items are more constrained, the item row can be
    // explicitly sized which should eliminate sizing instability.
    left: left - Math.min(obscuredMenuWidth, itemWidth - 21) - 4
  });

  // Move focus to the context menu
  currentContextMenu[0].focus();

  // Use the menuItemContainer which is where the items actually live.  This is
  // important so its calculated slopes are accurate and it can cancel change
  // timers when the mouse leaves the menu area.
  $(megaMenu.menuItemContainer).menuAim({
    rowSelector: '.context-menu-item',
    activate: function(row) {
      if (row.subMenuElem) {
        row.subMenuElem.style.visibility = 'visible';
      }
      row.classList.add('context-menu-maintain-hover');
    },
    deactivate: function(row) {
      if (row.subMenuElem) {
        row.subMenuElem.style.visibility = 'hidden';
      }
      row.classList.remove('context-menu-maintain-hover');
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
  chewQKinds(data.generated || []);

  return {
    decls: decls,
    defs: defs,
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
function makeDeclarationPopulater(sym) {
  return function(menuElem) {
    loadSymbolInfo(sym).then(function(info) {
      var fileResults;
      // This is a hack to support treating type definitions as declarations.
      if (info.decls.length) {
        fileResults = info.decls;
      } else {
        fileResults = info.defs;
      }
      fileResults.forEach(function(fileResult) {
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
            codeElem.textContent = line.rawComment + '\n' + line.line;
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
  // NOTE! We now ignore the "jumps" data.  The differences between "jumps" and
  // "searches" are:
  // - "jumps" stem from the crossref process sourced from "target" data,
  //   whereas "searches" are sourced from the "source" data.  The only
  //   difference from our perspective is the "source" "pretty" value includes
  //   the syntax kind as a prefix, so we get "type foo::Foo" and
  //   "constructor foo::Foo::Foo" instead of bare "foo::Foo" and
  //   "foo::Foo::Foo".
  // - Jumps are not emitted on their own source line, because indeed it would
  //   be silly to suggest jumping to the line you're already on.
  //
  // The extra syntax kind information is useful for our UI display purpose,
  // plus we always want to be able to expose the declaration, so we always
  // want to display the information regardless of whether jumping makes sense.
  var searches = ANALYSIS_DATA[index][1];

  var menuItems = [];

  /*
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
  */

  // For the header we want the most specific symbol we have.
  var longestPretty = '';
  for (var i = 0; i < searches.length; i++) {
    var sym = searches[i].sym;
    var pretty = searches[i].pretty;

    // Attempt to extract the syntaxKind
    var prettyParts = pretty.split(' ');
    var syntaxKind;
    if (prettyParts.length > 1) {
      syntaxKind = prettyParts[0];
      pretty = prettyParts[1];
    }

    if (pretty.length > longestPretty.length) {
      longestPretty = pretty;
    }

    var label;
    if (syntaxKind) {
      label = syntaxKind[0].toUpperCase() + syntaxKind.substring(1) +
                " declaration";
    } else {
      label = "Declaration";
    }

    // Display the declaration immediately.
    menuItems.push({
      label: label,
      href: `/${tree}/define?q=${encodeURIComponent(sym)}&redirect=false`,
      icon: "search",
      populateSubmenu: makeDeclarationPopulater(sym),
    });
    /*
    menuItems.push({
      label: label,
      href: `/${tree}/search?q=symbol:${encodeURIComponent(sym)}&redirect=false`,
      icon: "search",
      populateSubmenu: makeUsesSubmenuPopulater(sym),
    });
    */

    // Let uses be looked up asynchronously.
    // TODO
  }

  var menuHeader = { label: longestPretty };

  setContextMenu({ header: menuHeader, menuItems: menuItems }, event);
});
