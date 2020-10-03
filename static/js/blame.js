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

    // We play games with CSS variables to allow us to display a more detailed
    // set of colors when hovering over a coverage cell and a less detailed set
    // when not hovered.  See mozsearch.css for more info, but the basic idea
    // is:
    // - The "unhovered" CSS variables are used with preference in all our CSS
    //   styles, so when defined we use their less detailed colors.  So our
    //   style `background-color: var(--cov-miss-unhovered-color, #f7f7f7);`
    //   will use #f7f7f7 when it's not defined and the variable value when it
    //   is.
    // - So we set the "unhovered" values when we want a less detailed
    //   visualization of what's going on and we clear set them to the empty
    //   string.
    // -
    this.HIT_COLOR_VAR = "--cov-hit-color";
    this.MISS_COLOR_VAR = "--cov-miss-color";
    this.HIT_UNHOVERED_COLOR_VAR = "--cov-hit-unhovered-color";
    this.MISS_UNHOVERED_COLOR_VAR = "--cov-miss-unhovered-color";
    const computed = getComputedStyle(document.documentElement);
    this.COV_HIT_COLOR = computed.getPropertyValue(this.HIT_COLOR_VAR);
    this.COV_MISS_COLOR = computed.getPropertyValue(this.MISS_COLOR_VAR);

    this.coverageDetailsShown = true;
    this.hideCoverageStripDetails();
  }

  showCoverageStripDetails() {
    if (this.coverageDetailsShown) {
      return;
    }
    this.coverageDetailsShown = true;

    document.documentElement.style.setProperty(
      this.HIT_UNHOVERED_COLOR_VAR, "");
    document.documentElement.style.setProperty(
      this.MISS_UNHOVERED_COLOR_VAR, "");
  }

  hideCoverageStripDetails() {
    if (!this.coverageDetailsShown) {
      return;
    }
    this.coverageDetailsShown = false;

    document.documentElement.style.setProperty(
      this.HIT_UNHOVERED_COLOR_VAR, this.COV_HIT_COLOR);
    document.documentElement.style.setProperty(
      this.MISS_UNHOVERED_COLOR_VAR, this.COV_MISS_COLOR);
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
  // `blameElement`. The popup is added as a child of the blameElt in the DOM.
  async update() {
    // If there's no current element, just bail.
    if (!this.blameElement) {
      return this.hide();
    }

    // Latch the current element in case by the time our fetch comes back it's
    // no longer the current one.
    const elt = this.blameElement;
    // The coverage and annotate strips are adjacent and it would be bad UX for
    // hovering over the coverage strip to occlude the annotate strip, so we
    // adjust the coverage elements to use the annotate element for positioning.
    let hoverRightOfElt = elt;
    let content;
    const isAnnotate = !!elt.dataset.blame;
    if (isAnnotate) {
      content = await this.generateAnnotateContent(elt);
    } else {
      content = await this.generateCoverageContent(elt);
      // This obviously assumes the known hard-coded DOM from `format.rs`.
      hoverRightOfElt = elt.parentElement.nextElementSibling.firstElementChild;
    }

    // If no content was returned or the blame element has changed, bail.
    if (!content || this.blameElement != elt) {
      return;
    }

    let rect = hoverRightOfElt.getBoundingClientRect();
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

    // Adjust transform to ensure the popup doesn't go outside the window.
    let popupBox = this.popup.getBoundingClientRect();
    if (popupBox.bottom > window.innerHeight) {
      top -= (popupBox.bottom - window.innerHeight);
      this.popup.style.transform = `translatey(${top}px) translatex(${left}px)`;
    }

    if (isAnnotate) {
      this.hideCoverageStripDetails();
    } else {
      this.showCoverageStripDetails();
    }
  }

  async generateCoverageContent(elt) {
    let content;

    if (elt.classList.contains("cov-no-data")) {
      content =
        `<div>There is no coverage data for this file.</div>`;
    } else if (elt.classList.contains("cov-unknown")) {
      content =
        `<div>There was coverage data for this file but not for this line.</div>`;
    } else if (elt.classList.contains("cov-interpolated")) {
      content = `<div>This line wasn't instrumented for coverage, but we ` +
                `interpolated coverage for this line to make it visually less `+
                `distracting.</div>`;
    } else if (elt.classList.contains("cov-uncovered")) {
      content = `<div>This line wasn't instrumented for coverage.</div>`;
    } else {
      const hitCount = parseInt(elt.dataset.coverage, 10);
      content = `<div>This line was hit ${hitCount} times per coverage ` +
                `instrumentation.<div>`;
    }

    return content;
  }

  async generateAnnotateContent(elt) {
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

    return content;
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
    // Set to true if the user clicks on the blame strip. This will keep the
    // keep the popup visible until the user clicks elsewhere.
    this.keepVisible = false;

    for (let element of document.querySelectorAll(".blame-strip")) {
      element.addEventListener("mouseenter", this);
      element.addEventListener("mouseleave", this);
    }

    for (let element of document.querySelectorAll(".cov-strip")) {
      element.addEventListener("mouseenter", this);
      element.addEventListener("mouseleave", this);
    }

    BlamePopup.popup.addEventListener("mouseenter", this);
    BlamePopup.popup.addEventListener("mouseleave", this);
    // Click listener needs to be capturing since whatever is being clicked on
    // (e.g. a code fragment that displays a context menu) may actually
    // consume the event.
    window.addEventListener("click", this, { capture: true });
    document
      .getElementById("scrolling")
      .addEventListener("scroll", this, { passive: true });
  }

  handleEvent(event) {
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
      event.type == "click" && !event.target.matches(".blame-strip") &&
      !event.target.matches(".cov-strip");
    if (clickedOutsideBlameStrip && !BlamePopup.blameElement) {
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
        BlamePopup.blameElement = null;
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
        BlamePopup.blameElement = null;
        return
      }
      this.keepVisible = isClick;
      this.mouseElement = event.target;
      if (this.mouseElement != BlamePopup.popup) {
        BlamePopup.blameElement = this.mouseElement;
      }
    }
  }
})();
