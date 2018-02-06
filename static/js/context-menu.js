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

  $('body').append(nunjucks.render('static/templates/context-menu.html', menu));
  var currentContextMenu = $('#context-menu');

  currentContextMenu.css({
    top: top,
    left: left
  });

  // Move focus to the context menu
  currentContextMenu[0].focus();

  currentContextMenu.on('mousedown', function(event) {
    // Prevent clicks on the menu to propagate
    // to the window, so that the menu is not
    // removed and links will be followed.
    event.stopPropagation();
  });
}

// When this is set to true, moving the mouse doesn't change what is highlighted.
var stickyHover = false;

// Remove the menu when a user clicks outside it.
window.addEventListener('mousedown', function() {
  if (stickyHover) {
    stickyHover = false;
    hovered.removeClass("hovered");
    hovered = $();
  }
  $('#context-menu').remove();
}, false);

window.addEventListener("pageshow", function() {
  $('#context-menu').remove();
}, false);

var hovered = $();

$("#file").on("mousemove", function(event) {
  if ($('#context-menu').length || stickyHover) {
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
});

function stickyHighlight(id)
{
  $('#context-menu').remove();

  hovered.removeClass("hovered");
  hovered = $(`span[data-id="${id}"]`);
  hovered.addClass("hovered");

  stickyHover = true;
}

function getTargetWord()
{
  var selection = window.getSelection();
  if (!selection.isCollapsed) {
    return null;
  }

  var offset = selection.focusOffset;
  var node = selection.anchorNode;
  var selectedTxtString = node.nodeValue;
  var nonWordCharRE = /[^A-Z0-9_]/i;
  var startIndex = selectedTxtString.regexLastIndexOf(nonWordCharRE, offset) + 1;
  var endIndex = selectedTxtString.regexIndexOf(nonWordCharRE, offset);

  // If the regex did not find a start index, start from index 0
  if (startIndex === -1) {
    startIndex = 0;
  }

  // If the regex did not find an end index, end at the position
  // equal to the length of the string.
  if (endIndex === -1) {
    endIndex = selectedTxtString.length;
  }

  // If the offset is beyond the last word, no word was clicked on.
  if (offset > endIndex) {
    return null;
  }

  if (endIndex <= startIndex) {
    return null;
  }

  return selectedTxtString.substr(startIndex, endIndex - startIndex);
}

$("#file").on("click", "code", function(event) {
  stickyHover = false;

  var tree = $("#data").data("tree");

  function fmt(s, data) {
    data = data
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
    return s.replace("_", data);
  }

  var menuItems = [];

  var elt = $(event.target);
  var index = elt.closest("[data-i]").attr("data-i");
  if (index) {
    // Comes from the generated page.
    var [jumps, searches] = ANALYSIS_DATA[index];

    for (var i = 0; i < jumps.length; i++) {
      var sym = jumps[i].sym;
      var pretty = jumps[i].pretty;
      menuItems.push({html: fmt("Go to definition of _", pretty),
                      href: `/${tree}/define?q=${encodeURIComponent(sym)}&redirect=false`,
                      icon: "search"});
    }

    for (var i = 0; i < searches.length; i++) {
      var sym = searches[i].sym;
      var pretty = searches[i].pretty;
      menuItems.push({html: fmt("Search for _", pretty),
                      href: `/${tree}/search?q=symbol:${encodeURIComponent(sym)}&redirect=false`,
                      icon: "search"});
    }
  }

  var word = getTargetWord();
  if (word !== null) {
    // A word was clicked on.
    menuItems.push({html: fmt('Search for the substring <strong>_</strong>', word),
                    href: `/${tree}/search?q=${encodeURIComponent(word)}&redirect=false`,
                    icon: "search"});
  }

  var id = elt.closest("[data-id]").attr("data-id");
  if (id) {
    menuItems.push({html: "Sticky highlight",
                    href: `javascript:stickyHighlight('${id}')`});
  }

  if (menuItems.length > 0) {
    setContextMenu({menuItems: menuItems}, event);
  }
});
