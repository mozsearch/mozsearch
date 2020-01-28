var popupBox = null;

// The .blame-strip element for which blame is currently being displayed.
var blameElt;
// The .blame-strip element the mouse is hovering over.  Clicking on the element
// will null this out as a means of letting the user get rid of the popup if
// they didn't actually want to see the pop-up.
var mouseElt;

// A very simply MRU cache of size 1.  We don't issue an XHR if we already have
// the data.  This is important for the case where the user is moving their
// mouse along the same contiguous run of blame data.  In that case, the
// blameElt changes, but the `revs` stays the same.
var prevRevs;
var prevJson;

/**
 * Asynchronously initiates lookup and display of the blame data for the current
 * blameElt.  The popup is added as a child of the blameElt in the DOM.
 */
function updateBlamePopup() {
  // If there's no blameElt, just remove the popup if it exists and bail.
  if (!blameElt) {
    if (popupBox) {
      popupBox.remove();
      popupBox = null;
    }
    return;
  }

  // Latch the current blameElt in case by the time our XHR comes back it's no
  // longer the current blameElt.
  var elt = blameElt;
  var blame = elt.dataset.blame;

  var [revs, filespecs, linenos] = blame.split("#");
  var path = $("#data").data("path");
  var tree = $("#data").data("tree");

  function showPopup(json) {
    // If the XHR was too slow, we may no longer want to display blame for this
    // element, bail.
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
      rendered += `<br>Show <a href="${diffLink}">annotated diff</a>`;
      if (json[i].fulldiff) {
        rendered += ` or <a href="${json[i].fulldiff}">full diff</a>`;
      }

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
  // Suppress the blame hover popup if the context menu is visible.
  if ($('#context-menu').length) {
    return;
  }

  // Debounced pop-up closer.  If the mouse leaves a blame-strip element and
  // doesn't move onto another one within 100ms, close the popup.  Also, if the
  // user clicks on the blame-strip element and doesn't move onto a new element,
  // close the pop-up.
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
    // This is a mouseenter event.  Because the popup is a child of the
    // .blame-strip element, we may be deep inside popup content if the popup is
    // visible.  So we walk upwards until we find either a .blame-strip element
    // with data-blame set (which may be a new blameElt) or the existing popup,
    // in which case we can reuse the blameElt.
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
