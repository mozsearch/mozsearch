var popupBox = null;

var blameElt;
var mouseElt;

var prevRevs;
var prevJson;

function updateBlamePopup() {
  if (!blameElt) {
    if (popupBox) {
      popupBox.remove();
      popupBox = null;
    }
    return;
  }

  var elt = blameElt;
  var blame = elt.dataset.blame;

  var [revs, filespecs, linenos] = blame.split("#");
  var path = $("#data").data("path");
  var tree = $("#data").data("tree");

  function showPopup(json) {
    if (blameElt != elt) {
      return;
    }

    if (popupBox) {
      popupBox.remove();
      popupBox = null;
    }

    var content = "";

    var revList = revs.split(',');
    var filespecList = filespecs.split(',');
    var linenoList = linenos.split(',');

    // The last entry in the list (if it's not empty) is the real one we want
    // to show. The entries before that were "ignored", so we put them in a
    // hidden box that the user can expand.
    var ignored = [];
    for (var i = 0; i < revList.length; i++) {
      if (revList[i] == '') {
          // An empty final entry is used to indicate that all
          // the entries we provided were "ignored" (but we didn't
          // provide more because we hit the max limit)
          break;
      }

      var rendered = '';
      var revPath = filespecList[i] == "%" ? path : filespecList[i];
      rendered += `<div class="blame-entry">`;
      rendered += json[i].header;

      var diffLink = `/${tree}/diff/${revList[i]}/${revPath}#${linenoList[i]}`;
      rendered += `<br><a href="${diffLink}">Show annotated diff</a>`;

      if (json[i].parent) {
        var parentLink = `/${tree}/rev/${json[i].parent}/${revPath}#${linenoList[i]}`;
        rendered += `<br><a href="${parentLink}" class="deemphasize">Show latest version without this line</a>`;
      }

      var revLink = `/${tree}/rev/${revList[i]}/${revPath}#${linenoList[i]}`;
      rendered += `<br><a href="${revLink}" class="deemphasize">Show earliest version with this line</a>`;
      rendered += '</div>';

      if (i < revList.length - 1) {
        ignored.push(rendered);
      } else {
        content += rendered;
      }
    }
    if (ignored.length > 0) {
      content += `<br><details><summary>${ignored.length} ignored changesets</summary>${ignored.join("")}</details>`;
    }

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

    prevRevs = revs;
    prevJson = response;
  }

  if (prevRevs == revs) {
    showPopup(prevJson);
  } else {
    var req = new XMLHttpRequest();
    req.addEventListener("load", reqListener);
    req.open("GET", `/${tree}/commit-info/${revs}`);
    req.send();
  }
}

function setBlameElt(elt) {
  if (blameElt == elt) {
    return;
  }
  if (blameElt) {
    blameElt.setAttribute("aria-expanded", false);
  }
  blameElt = elt;
  if (blameElt) {
    blameElt.setAttribute("aria-expanded", true);
  }
}

function blameHoverHandler(event) {
  if ($('#context-menu').length) {
    return;
  }

  if (event.type == "mouseleave" ||
      (event.type == "click" && blameElt != null)) {
    mouseElt = null;

    setTimeout(function() {
      if (!mouseElt) {
        setBlameElt(null);
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

    setBlameElt(mouseElt);
    updateBlamePopup();
  }
}

$(".blame-strip").on("mouseenter", blameHoverHandler);
$(".blame-strip").on("mouseleave", blameHoverHandler);
$(".blame-strip").on("click", blameHoverHandler);

