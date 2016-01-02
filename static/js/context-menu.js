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
  var kind = elt.attr("data-kind");

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
  var id = elt.attr("data-id");
  var idName = elt.text();
  var extra = elt.attr("data-extra");
  var kind = elt.attr("data-kind");
  var jump = elt.attr("data-jump") == "true";
  var extraJump = elt.attr("data-extra-jump") == "true";

  function fmt(s, data) {
    return s.replace("_", "&ldquo;" + data + "&rdquo;");
  }

  var propName;
  if (id.startsWith("#")) {
    propName = id;
  } else {
    propName = idName;
  }

  var menuItems = [];

  var extraName;
  if (extra) {
    extraName = extra.replace("#", ".");
  }

  if (extraJump) {
    menuItems.push({html: fmt("Goto definition of _", extraName),
                    href: "/mozilla-central/define?q=" + encodeURIComponent(extra),
                    icon: "search"});
  }
  if (jump) {
    menuItems.push({html: fmt("Goto definition of _", idName),
                    href: "/mozilla-central/define?q=" + encodeURIComponent(id),
                    icon: "search"});
  }

  if (id.startsWith("#")) {
    if (extra) {
      menuItems.push({html: fmt("Search for property _", extraName),
                      href: "/mozilla-central/search?q=symbol:" + encodeURIComponent(extra),
                      icon: "search"});
    }
    menuItems.push({html: fmt("Search for property _", idName),
                    href: "/mozilla-central/search?q=symbol:" + encodeURIComponent(id),
                    icon: "search"});
  } else {
    menuItems.push({html: fmt("Search for _", idName),
                    href: "/mozilla-central/search?q=symbol:" + encodeURIComponent(id),
                    icon: "search"});
  }

  setContextMenu({menuItems: menuItems}, event);
});
