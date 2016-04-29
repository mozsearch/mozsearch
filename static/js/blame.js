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

    popupBox.style.border = "1px solid black";
    popupBox.style.position = "absolute";
    popupBox.style.top = parseInt(height) + "px";
    popupBox.style.left = 0;
    popupBox.style.padding = "10px";
    popupBox.style.background = "white";
    popupBox.style["z-index"] = 100;

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
