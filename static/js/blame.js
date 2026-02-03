/**
 * The BlamePopup is used for showing coverage and "annotate" (the less
 * judgemental term for "blame").  It previously was "annotate" specific.  Out
 * of an abundance of laziness and to minimize diff size, the existing
 * terminology is being left intact for now.
 */
var BlamePopup = new (class BlamePopup {
  constructor() {
    this.popup = document.createElement("div");
    this.popup.id = "blame-popup";
    this.popup.className = "blame-popup";
    this.popup.style.display = "none";
    document.body.appendChild(this.popup);

    // The .cov-strip, .blame-strip, macro or gc button for which the popup is currently being displayed.
    this._triggerElement = null;
    this._expansionIndex = null;

    // The previous blame element for which we have already shown the popup.
    //
    // This is the current owner of the popup element.
    this.popupOwner = null;

    // A very simply MRU cache of size 1.  We don't issue an XHR if we already
    // have the data.  This is important for the case where the user is moving
    // their mouse along the same contiguous run of blame data.  In that case,
    // the `triggerElement` changes, but the `revs` stays the same.
    this.prevRevs = null;
    this.prevJson = null;

    this.coverageDetailsShown = true;
    this.hideCoverageStripDetails();
  }

  showCoverageStripDetails() {
    if (this.coverageDetailsShown) {
      return;
    }
    this.coverageDetailsShown = true;
    document.documentElement.classList.add("coverage-details-shown");
  }

  hideCoverageStripDetails() {
    if (!this.coverageDetailsShown) {
      return;
    }
    this.coverageDetailsShown = false;
    document.documentElement.classList.remove("coverage-details-shown");
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

    this.hideCoverageStripDetails();
  }

  // Asynchronously initiates lookup and display of the blame data for the current
  // `triggerElement`. The popup is added as a child of the blameElt in the DOM.
  async update() {
    // If there's no current element, just bail.
    if (!this.triggerElement) {
      this.hide();
      return;
    }

    const elt = this.triggerElement;
    let content;

    let top;
    let left;

    let isGC = false;
    let gcInfo = null;
    if (Settings.semanticInfo.enabled) {
      if (elt?.dataset?.symbols) {
        for (const sym of elt.dataset.symbols.split(",")) {
          if (sym in SYM_INFO) {
            const info = SYM_INFO[sym];
            if (info.meta && "canGC" in info.meta) {
              gcInfo = info;
              isGC = true;
            }
          }
        }
      }
    }

    const isExpansion = typeof elt.dataset.expansions !== 'undefined' && elt.dataset.expansions !== null;
    if (isExpansion) {
      content = await this.generateExpansionContent(elt);
      let rect = elt.getBoundingClientRect();
      top = rect.bottom + window.scrollY;
      left = rect.left + window.scrollX;
    } else if (isGC) {
      if (gcInfo.meta.canGC) {
        content = "This function can GC in the following path.<code>";
        content += gcInfo.meta.gcPath.split("\n").map(x => "> " + x).join("\n")
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;")
          .replace(/"/g, "&quot;")
          .replace(/'/g, "&#039;");
        content += "</code>";
      } else {
        content = "This function cannot GC.";
      }
      let rect = elt.getBoundingClientRect();
      top = rect.bottom + window.scrollY;
      left = rect.left + window.scrollX;
    } else {
      // this.triggerElement can be the .cov-strip or .blame-strip element.
      // Get their parent .line-strip and find the other one.
      const lineElt = this.triggerElement.closest(".line-strip");
      const covElt = lineElt.querySelector(".cov-strip");
      const blameElt = lineElt.querySelector(".blame-strip");

      content = "";
      content += await this.generateCoverageContent(covElt);
      content += "<hr>";
      content += await this.generateAnnotateContent(blameElt);

      let rect = lineElt.getBoundingClientRect();
      top = rect.top + window.scrollY;
      left = rect.right + window.scrollX;
    }

    // If no content was returned or the trigger element has changed, bail.
    if (!content || this.triggerElement != elt) {
      return;
    }

    this.detachFromCurrentOwner();
    this.popup.style.display = "";
    // This also works, but transform doesn't even require layout.
    // this.popup.style.left = left + "px";
    // this.popup.style.top = top + "px";
    this.popup.style.transform = `translatey(${top}px) translatex(${left}px)`;
    this.popup.innerHTML = content;
    this.popupOwner = this.triggerElement;
    // We set aria-owns on the parent role=cell instead of the button.
    this.popupOwner.parentNode.setAttribute("aria-owns", "blame-popup");
    this.popupOwner.setAttribute("aria-expanded", "true");

    // Adjust transform to ensure the popup doesn't go outside the window.
    let popupBox = this.popup.getBoundingClientRect();
    if (popupBox.bottom > window.innerHeight) {
      top -= popupBox.bottom - window.innerHeight;
      this.popup.style.transform = `translatey(${top}px) translatex(${left}px)`;
    }

    if (!isExpansion && !isGC) {
      this.showCoverageStripDetails();
    }
  }

  async generateExpansionContent(elt) {
    const expansions = JSON.parse(elt.dataset.expansions);
    const sym = this.expansionIndex[0];
    const platform = this.expansionIndex[1];
    const jumpref = this.expansionIndex[2];
    const expansion = expansions[sym][platform];
    const onlyOneExpansion = Object.keys(expansions).length == 1 && Object.keys(expansions[sym]).length == 1;

    if (jumpref && jumpref.jumps.def) {
      const tree = document.getElementById("data").getAttribute("data-tree");
      const jumpFileName = jumpref.jumps.def.slice(jumpref.jumps.def.lastIndexOf(',') + 1)
      if (onlyOneExpansion) {
        return `Expansion of <span class="symbol" data-symbols="${sym}">${jumpref.pretty}</span>:<br><code>${expansion}</code>`;
      } else {
        return `Expansion of <span class="symbol" data-symbols="${sym}">${jumpref.pretty}</span> on ${platform}:<br><code>${expansion}</code>`;
      }
    } else {
      if (onlyOneExpansion) {
        return `Expansion:<br><code>${expansion}</code>`;
      } else {
        return `Expansion on ${platform}:<br><code>${expansion}</code>`;
      }
    }
  }

  async generateCoverageContent(elt) {
    let content = `<div class="coverage-entry">`;

    if (elt.classList.contains("cov-no-data")) {
      content = `There is no coverage data for this file.`;
    } else if (elt.classList.contains("cov-unknown")) {
      content = `There was coverage data for this file but not for this line.`;
    } else if (elt.classList.contains("cov-interpolated")) {
      content =
        `This line wasn't instrumented for coverage, but we ` +
        `interpolated coverage for this line to make it visually less ` +
        `distracting.`;
    } else if (elt.classList.contains("cov-uncovered")) {
      content = `This line wasn't instrumented for coverage.`;
    } else {
      const hitCount = parseInt(elt.dataset.coverage, 10);
      content =
        `This line was hit ${hitCount} times per coverage ` +
        `instrumentation.`;
    }

    const makeLink = (revision) =>  {
      const data = document.getElementById("data");
      const path = data.dataset.path;
      const tree = data.dataset.tree;
      return `/${tree}/rev/${revision}/${path}`;
    };

    const data = document.getElementById("coverage-navigation").dataset;
    if ("previous" in data)
      content += `<br><a href="${makeLink(data.previous)}">Show previous file revision with coverage</a>`;
    if ("next" in data)
      content += `<br><a href="${makeLink(data.next)}">Show next file revision with coverage</a>`;
    if ("latest" in data)
      content += `<br><a href="${makeLink(data.latest)}">Show latest file revision with coverage</a>`;

    content += `</div>`;

    return content;
  }

  async generateAnnotateContent(elt) {
    const blame = elt.dataset.blame;
    const [revs, filespecs, linenos] = blame.split("#");

    const data = document.getElementById("data");
    const path = data.getAttribute("data-path");
    const tree = data.getAttribute("data-tree");

    // Latch the current element in case by the time our fetch comes back it's
    // no longer the current one.
    const triggerElement = this.triggerElement;

    if (this.prevRevs != revs) {
      let response = await fetch(`/${tree}/commit-info/${revs}`);
      this.prevJson = await response.json();
      this.prevRevs = revs;
    }

    // If the request was too slow, we may no longer want to display blame for
    // this element, bail.
    if (this.triggerElement != triggerElement) {
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
      rendered += `<br>Show <a href="${encodeURI(diffLink)}">annotated diff</a>`;

      if (json[i].fulldiff) {
        rendered += ` or <a href="${encodeURI(json[i].fulldiff)}">full diff</a>`;
      }

      if (json[i].phab) {
        let name = "Phabricator revision";
        const m = json[i].phab.match(/\/(D[0-9]+)/);
        if (m) {
          name += " " + m[1];
        }
        rendered += ` or <a href="${encodeURI(json[i].phab)}">${name}</a>`;
      }

      if (json[i].pr) {
        let name = "Pull request";
        const m = json[i].pr.match(/\/([0-9]+)/);
        if (m) {
          name += " #" + m[1];
        }
        rendered += ` or <a href="${encodeURI(json[i].pr)}">${name}</a>`;
      }

      if (json[i].parent) {
        let parentLink = `/${tree}/rev/${json[i].parent}/${revPath}#${linenoList[i]}`;
        rendered += `<br><a href="${encodeURI(parentLink)}" class="deemphasize">Show latest version without this line</a>`;
      }

      let revLink = `/${tree}/rev/${revList[i]}/${revPath}#${linenoList[i]}`;
      rendered += `<br><a href="${encodeURI(revLink)}" class="deemphasize">Show earliest version with this line</a>`;
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

    return content;
  }

  get triggerElement() {
    return this._triggerElement;
  }

  set triggerElement(newElement) {
    if (this.triggerElement == newElement) {
      return;
    }
    this._triggerElement = newElement;
    this.update();
  }

  get expansionIndex() {
    return this._expansionIndex;
  }

  set expansionIndex(value) {
    if (this.expansionIndex == value) {
      return;
    }
    this._expansionIndex = value;
    this.update();
  }
})();

var BlameStripHoverHandler = new (class BlameStripHoverHandler {
  constructor() {
    // The .blame-strip element the mouse is hovering over.  Clicking on the
    // element will null this out as a means of letting the user get rid of the
    // popup if they didn't actually want to see the pop-up.
    this.mouseElement = null;
    // Set to true if the user clicks on the blame strip. This will keep the
    // keep the popup visible until the user clicks elsewhere.
    this.keepVisible = false;

    for (let element of document.querySelectorAll(".blame-strip")) {
      element.addEventListener("mouseenter", this);
      element.addEventListener("mouseleave", this);
      // Use passive listeners for touch, because these elements already
      // disable touch-panning via touch-action properties.
      element.addEventListener("touchstart", this, { passive: true });
      element.addEventListener("touchmove", this, { passive: true });
    }

    for (let element of document.querySelectorAll(".cov-strip")) {
      element.addEventListener("mouseenter", this);
      element.addEventListener("mouseleave", this);
      // Use passive listeners for touch, because these elements already
      // disable touch-panning via touch-action properties.
      element.addEventListener("touchstart", this, { passive: true });
      element.addEventListener("touchmove", this, { passive: true });
    }

    BlamePopup.popup.addEventListener("mouseenter", this);
    BlamePopup.popup.addEventListener("mouseleave", this);
    // Click listener needs to be capturing since whatever is being clicked on
    // (e.g. a code fragment that displays a context menu) may actually
    // consume the event.
    window.addEventListener("click", this, { capture: true });
    window.addEventListener("scroll", this, { passive: true });
  }

  isStripElement(elem) {
    return elem.matches(".blame-strip") || elem.matches(".cov-strip");
  }

  handleEvent(event) {
    if (event.type == "touchstart" || event.type == "touchmove") {
      // For touch events, event.target is always the element at which the first touchstart landed. So
      // this condition filters for touch sequences where the touchstart started on a strip element.
      if (this.isStripElement(event.target) && event.touches.length == 1) {
        // Within those touch sequences, update the blame element to whatever is under the touch
        // point, or null if the touch point moves off the strip.
        let elementUnderTouch = document.elementFromPoint(
          event.touches[0].clientX,
          event.touches[0].clientY
        );
        BlamePopup.triggerElement = this.isStripElement(elementUnderTouch)
          ? elementUnderTouch
          : null;
      }
      return;
    }

    // Suppress the blame hover popup if the context menu is visible.
    if (ContextMenu.active) {
      return;
    }

    if (this.keepVisible) {
      if (event.type == "mouseenter" || event.type == "mouseleave") {
        // Ignore mouseenter/mouseleave events if keepVisible is true
        return;
      }
    }

    let clickedOutsideBlameStrip =
      event.type == "click" && !this.isStripElement(event.target);
    if (clickedOutsideBlameStrip && !BlamePopup.triggerElement) {
      // Don't care about clicks outside the blame strip if there's no popup showing.
      return;
    }
    if (clickedOutsideBlameStrip && BlamePopup.popup.contains(event.target)) {
      // Also don't care if the click landed on the blame popup itself (e.g. clicking
      // on a link or expanding the details box in the popup).
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
      event.type == "scroll" ||
      clickedOutsideBlameStrip
    ) {
      this.keepVisible = false;
      this.mouseElement = null;
      setTimeout(() => {
        if (this.mouseElement) {
          return;
        } // Mouse moved somewhere else inside the strip.
        BlamePopup.triggerElement = null;
      }, 100);
    } else {
      // We run this code on either "mouseenter", or on a "click" event where
      // the click landed inside the blame strip. In the latter case we set
      // keepVisible to pin the blame popup open until another click dismisses
      // it, or a scroll event happens (since the popup doesn't move properly with
      // scrolling).
      const isClick = event.type == "click";
      // A click on the same exact element as the last click should toggle the
      // popup closed.  This is important for the screen-reader use-case where
      // inspecting blame details involves hitting enter on the role=button
      // blame cell to toggle the blame popup on and then off.
      //
      // We could check `keepVisible` here but that creates a weird situation
      // when activating via mouse where we get the sequence 1) hover triggers
      // popup, 2) click maintains popup and makes it keepVisible, and then 3)
      // second click hides popup.  By not checking, we get 1) hover triggers
      // popup, 2) click hides popup.
      if (isClick && this.mouseElement === event.target) {
        this.keepVisible = false;
        this.mouseElement = null;
        BlamePopup.triggerElement = null;
        return;
      }
      this.keepVisible = isClick;
      this.mouseElement = event.target;
      if (this.mouseElement != BlamePopup.popup) {
        BlamePopup.triggerElement = this.mouseElement;
      }
    }
  }
})();
