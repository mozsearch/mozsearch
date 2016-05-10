var popupBox = null;

function onClick(event) {
  if (event.target.className != "blame") {
    var elt = event.target;
    while (elt) {
      if (elt.className == "blame") {
        return;
      }
      elt = elt.parentNode;
    }
    if (popupBox) {
      popupBox.remove();
      popupBox = null;
    }

    return;
  }

  event.preventDefault();

  var rev = event.target.dataset.rev;
  var link = event.target.dataset.link;

  function reqListener() {
    var response = JSON.parse(this.responseText);

    if (popupBox) {
      popupBox.remove();
    }

    var height = event.target.getBoundingClientRect().height;

    var content = response.header;
    content += '<br><a href="' + link + '">Diff</a>';

    popupBox = document.createElement("div");
    popupBox.innerHTML = content;

    popupBox.className = "blame-popup";

    event.target.appendChild(popupBox);
    event.target.style.position = "relative";
  }

  var req = new XMLHttpRequest();
  req.addEventListener("load", reqListener);
  req.open("GET", "/mozilla-central/commit-info/" + rev);
  req.send();
  return;

}

function onLoad() {
  document.addEventListener("click", onClick, true);
}

window.addEventListener("load", onLoad);





var prev_elt;
var hovered = $();

$("#line-numbers").on("mousemove", function(event) {
  if ($('#context-menu').length) {
    return;
  }

  var y = event.clientY;
  var x = event.clientX;

  var elt = document.elementFromPoint(x, y);
  while (!elt.hasAttribute("data-rev")) {
    elt = elt.parentNode;
    if (!elt || !(elt instanceof Element)) {
      hovered = $();
      return;
    }
  }

  if (prev_elt == elt) {
    return;
  }
  prev_elt = elt;

  //elt = $(elt);
  var rev = elt.dataset.rev;
  var link = elt.dataset.link;
  var strip = elt.dataset.strip;

  if (popupBox) {
    popupBox.remove();
    popupBox = null;
  }

  function reqListener() {
    var response = JSON.parse(this.responseText);
    var content = response.header;
    content += '<br><a href="' + link + '">Diff</a>';

    hovered = $("div[data-strip=" + strip + "]");
    hovered.addClass("hovered2");
    hovered.removeClass("hovered2");

    var parent = elt.parentNode;

    var height = elt.getBoundingClientRect().height;

    popupBox = document.createElement("div");
    popupBox.innerHTML = content;
    popupBox.className = "blame-popup";
    parent.appendChild(popupBox);
  }

  var req = new XMLHttpRequest();
  req.addEventListener("load", reqListener);
  req.open("GET", "/mozilla-central/commit-info/" + rev);
  req.send();
  return;
});
