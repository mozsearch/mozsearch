var BlamePopup = new (class BlamePopup {
  constructor() {
    this.popup = document.createElement("div");
    this.popup.id = "blame-popup";
    this.popup.className = "blame-popup";
    this.popup.style.display = "none";
    document.documentElement.appendChild(this.popup);

    // The .blame-strip element for which blame is currently being displayed.
    this._blameElement = null;

    // The previous blame element for which we have already shown the popup.
    //
    // This is the current owner of the popup element.
    this.popupOwner = null;

    // A very simply MRU cache of size 1.  We don't issue an XHR if we already
    // have the data.  This is important for the case where the user is moving
    // their mouse along the same contiguous run of blame data.  In that case,
    // the `blameElement` changes, but the `revs` stays the same.
    this.prevRevs = null;
    this.prevJson = null;
  }

  detachFromCurrentOwner() {
    if (!this.popupOwner) {
      return;
    }
    this.popupOwner.parentNode.removeAttribute("aria-owns");
    this.popupOwner.setAttribute("aria-expanded", "false");
    this.popupOwner = null;
  }

  // Hides the popup if open.
  hide() {
    this.detachFromCurrentOwner();
    this.popup.style.display = "none";
  }

  // Asynchronously initiates lookup and display of the blame data for the current
  // `blameElement`. The popup is added as a child of the blameElt in the DOM.
  async update() {
    // If there's no current element, just bail.
    if (!this.blameElement) {
      return this.hide();
    }

    // Latch the current element in case by the time our fetch comes back it's
    // no longer the current one.
    const elt = this.blameElement;
    const blame = elt.dataset.blame;
    const [revs, filespecs, linenos] = blame.split("#");

    const data = document.getElementById("data");
    const path = data.getAttribute("data-path");
    const tree = data.getAttribute("data-tree");

    if (this.prevRevs != revs) {
      let response = await fetch(`/${tree}/commit-info/${revs}`);
      this.prevJson = await response.json();
      this.prevRevs = revs;
    }

    // If the request was too slow, we may no longer want to display blame for
    // this element, bail.
    if (this.blameElement != elt) {
      return;
    }

    let json = this.prevJson;

    let content = "";
    let revList = revs.split(",");
    let filespecList = filespecs.split(",");
    let linenoList = linenos.split(",");

    // The last entry in the list (if it's not empty) is the real one we want
    // to show. The entries before that were "ignored", so we put them in a
    // hidden box that the user can expand.
    let ignored = [];
    for (let i = 0; i < revList.length; i++) {
      // An empty final entry is used to indicate that all the entries we
      // provided were "ignored" (but we didn't provide more because we hit the
      // max limit).
      if (!revList[i]) {
        break;
      }

      let rendered = "";
      let revPath = filespecList[i] == "%" ? path : filespecList[i];
      rendered += `<div class="blame-entry">`;
      rendered += json[i].header;

      let diffLink = `/${tree}/diff/${revList[i]}/${revPath}#${linenoList[i]}`;
      rendered += `<br>Show <a href="${diffLink}">annotated diff</a>`;

      if (json[i].fulldiff) {
        rendered += ` or <a href="${json[i].fulldiff}">full diff</a>`;
      }

      if (json[i].parent) {
        let parentLink = `/${tree}/rev/${json[i].parent}/${revPath}#${linenoList[i]}`;
        rendered += `<br><a href="${parentLink}" class="deemphasize">Show latest version without this line</a>`;
      }

      let revLink = `/${tree}/rev/${revList[i]}/${revPath}#${linenoList[i]}`;
      rendered += `<br><a href="${revLink}" class="deemphasize">Show earliest version with this line</a>`;
      rendered += "</div>";

      if (i < revList.length - 1) {
        ignored.push(rendered);
      } else {
        content += rendered;
      }
    }

    if (ignored.length) {
      content += `<br><details><summary>${
        ignored.length
      } ignored changesets</summary>${ignored.join("")}</details>`;
    }

    let rect = this.blameElement.getBoundingClientRect();
    let top = rect.top + window.scrollY;
    let left = rect.left + rect.width + window.scrollX;

    this.detachFromCurrentOwner();
    this.popup.style.display = "";
    // This also works, but transform doesn't even require layout.
    // this.popup.style.left = left + "px";
    // this.popup.style.top = top + "px";
    this.popup.style.transform = `translatey(${top}px) translatex(${left}px)`;
    this.popup.innerHTML = content;
    this.popupOwner = this.blameElement;
    // We set aria-owns on the parent role=cell instead of the button.
    this.popupOwner.parentNode.setAttribute("aria-owns", "blame-popup");
    this.popupOwner.setAttribute("aria-expanded", "true");
  }

  get blameElement() {
    return this._blameElement;
  }

  set blameElement(newElement) {
    if (this.blameElement == newElement) {
      return;
    }
    this._blameElement = newElement;
    this.update();
  }
})();

var BlameStripHoverHandler = new (class BlameStripHoverHandler {
  constructor() {
    // The .blame-strip element the mouse is hovering over.  Clicking on the
    // element will null this out as a means of letting the user get rid of the
    // popup if they didn't actually want to see the pop-up.
    this.mouseElement = null;

    for (let element of document.querySelectorAll(".blame-strip")) {
      element.addEventListener("mouseenter", this);
      element.addEventListener("mouseleave", this);
      element.addEventListener("click", this);
    }

    BlamePopup.popup.addEventListener("mouseenter", this);
    BlamePopup.popup.addEventListener("mouseleave", this);
  }

  handleEvent(event) {
    // Suppress the blame hover popup if the context menu is visible.
    if (ContextMenu.active) {
      return;
    }
    // Debounced pop-up closer..
    //
    // If the mouse leaves a blame-strip element and doesn't move onto another
    // one within 100ms, close the popup.  Also, if the user clicks on the
    // blame-strip element and doesn't move onto a new element, close the
    // pop-up.
    if (
      event.type == "mouseleave" ||
      (event.type == "click" && BlamePopup.blameElement)
    ) {
      this.mouseElement = null;
      setTimeout(() => {
        if (this.mouseElement) {
          return;
        } // Mouse moved somewhere else inside the strip.
        BlamePopup.blameElement = null;
      }, 100);
    } else {
      this.mouseElement = event.target;
      if (this.mouseElement != BlamePopup.popup) {
        BlamePopup.blameElement = this.mouseElement;
      }
    }
  }
})();
