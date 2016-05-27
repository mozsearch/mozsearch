var popupBox = null;

var blameElt;
var mouseElt;

var prevRev;
var prevContent;

function updateBlamePopup() {
  if (popupBox) {
    popupBox.remove();
    popupBox = null;
  }

  if (!blameElt) {
    return;
  }

  var elt = blameElt;
  var rev = elt.dataset.rev;
  var link = elt.dataset.link;
  var strip = elt.dataset.strip;

  function showPopup(content) {
    if (blameElt != elt) {
      return;
    }

    content += '<br><a href="' + link + '">Show annotated diff</a>';

    var parent = blameElt.parentNode;
    var height = blameElt.getBoundingClientRect().height;

    popupBox = document.createElement("div");
    popupBox.id = "blame-popup";
    popupBox.innerHTML = content;
    popupBox.className = "blame-popup";
    parent.appendChild(popupBox);

    $(popupBox).on("mouseenter", blameHoverHandler);
    $(popupBox).on("mouseleave", blameHoverHandler);
  }

  function reqListener() {
    var response = JSON.parse(this.responseText);
    var content = response.header;
    showPopup(content);

    prevRev = rev;
    prevContent = content;
  }

  if (prevRev == rev) {
    showPopup(prevContent);
  } else {
    var req = new XMLHttpRequest();
    req.addEventListener("load", reqListener);
    req.open("GET", "/mozilla-central/commit-info/" + rev);
    req.send();
  }
}

function blameHoverHandler(event) {
  if ($('#context-menu').length) {
    return;
  }

  if (event.type == "mouseleave") {
    mouseElt = null;

    setTimeout(function() {
      if (!mouseElt) {
        blameElt = null;
        updateBlamePopup();
      }
    }, 100);
  } else {
    var elt = event.target;
    while (elt && elt instanceof Element) {
      if (elt.hasAttribute("data-rev")) {
        mouseElt = elt;
        break;
      }
      if (elt.id == "blame-popup") {
        mouseElt = blameElt;
        return;
      }
      elt = elt.parentNode;
    }
    if (!elt || !(elt instanceof Element)) {
      return;
    }

    blameElt = mouseElt;
    updateBlamePopup();
  }
}

$(".blame-strip").on("mouseenter", blameHoverHandler);
$(".blame-strip").on("mouseleave", blameHoverHandler);

