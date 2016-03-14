function setContextMenu(menu, event)
{
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

// Remove the menu when a user clicks outside it.
window.addEventListener('mousedown', function() {
  //toggleSymbolHighlights();
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
});

$("#file").on("click", "span[data-id]", function(event) {
  var elt = $(event.target);
  while (!elt.attr("data-id")) {
    elt = elt.parent();
  }
  var index = elt.attr("data-i");

  function fmt(s, data) {
    return s.replace("_", data);
  }

  // Comes from the generated page.
  var [jumps, searches] = ANALYSIS_DATA[index];

  var menuItems = [];

  for (var i = 0; i < jumps.length; i++) {
    var sym = jumps[i].sym;
    var pretty = jumps[i].pretty;
    menuItems.push({html: fmt("Goto definition of _", pretty),
                    href: "/mozilla-central/define?q=" + encodeURIComponent(sym),
                    icon: "search"});
  }

  for (var i = 0; i < searches.length; i++) {
    var sym = searches[i].sym;
    var pretty = searches[i].pretty;
    menuItems.push({html: fmt("Search for _", pretty),
                    href: "/mozilla-central/search?q=symbol:" + encodeURIComponent(sym),
                    icon: "search"});
  }

  setContextMenu({menuItems: menuItems}, event);
});
