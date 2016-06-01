var popupBox = null;

var blameElt;
var mouseElt;

var prevRev;
var prevJson;

function updateBlamePopup() {
  if (popupBox) {
    popupBox.remove();
    popupBox = null;
  }

  if (!blameElt) {
    return;
  }

  var elt = blameElt;
  var blame = elt.dataset.blame;

  var [rev, filespec, lineno] = blame.split("#");
  var path = $("#data").data("path");
  var tree = $("#data").data("tree");
  if (filespec != "%") {
    path = filespec;
  }

  function showPopup(json) {
    if (blameElt != elt) {
      return;
    }

    var content = json.header;

    var diffLink = `/${tree}/diff/${rev}/${path}#${lineno}`;
    content += `<br><a href="${diffLink}">Show annotated diff</a>`;

    if (json.parent) {
      var parentLink = `/${tree}/rev/${json.parent}/${path}#${lineno}`;
      content += `<br><a href="${parentLink}" class="deemphasize">Show latest version without this line</a>`;
    }

    var revLink = `/${tree}/rev/${rev}/${path}#${lineno}`;
    content += `<br><a href="${revLink}" class="deemphasize">Show earliest version with this line</a>`;

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
    showPopup(response);

    prevRev = rev;
    prevJson = response;
  }

  if (prevRev == rev) {
    showPopup(prevJson);
  } else {
    var req = new XMLHttpRequest();
    req.addEventListener("load", reqListener);
    req.open("GET", `/${tree}/commit-info/${rev}`);
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
      if (elt.hasAttribute("data-blame")) {
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

