var popupBox = null;

var blameElt;
var mouseElt;

var prevRev;
var prevContent;

var pendingReq;

function updateBlamePopup() {
  if (popupBox) {
    pending_req = null;
    popupBox.remove();
    popupBox = null;
  }

  if (!blameElt) {
    return;
  }

  var rev = blameElt.dataset.rev;
  var link = blameElt.dataset.link;
  var strip = blameElt.dataset.strip;

  function showPopup(content) {
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
    if (!pendingReq) {
      return;
    }
    pendingReq = null;

    var response = JSON.parse(this.responseText);
    var content = response.header;
    showPopup(content);

    prevRev = rev;
    prevContent = content;
  }

  if (prevRev == rev) {
    showPopup(prevContent);
  } else if (!pendingReq) {
    var req = new XMLHttpRequest();
    pendingReq = req;
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
      if (!elt || !(elt instanceof Element)) {
        return;
      }
    }

    blameElt = mouseElt;
    updateBlamePopup();
  }
}

$(".blame-strip").on("mouseenter", blameHoverHandler);
$(".blame-strip").on("mouseleave", blameHoverHandler);

