var ContextMenu = new (class ContextMenu {
  constructor() {
    this.menu = document.createElement("ul");
    this.menu.className = this.menu.id = "context-menu";
    this.menu.tabIndex = 0;
    this.menu.style.display = "none";
    document.body.appendChild(this.menu);

    this.menu.addEventListener("mousedown", function (event) {
      // Prevent clicks on the menu to propagate
      // to the window, so that the menu is not
      // removed and links will be followed.
      event.stopPropagation();
    });

    window.addEventListener("mousedown", () => this.hide());
    window.addEventListener("pageshow", () => this.hide());
    window.addEventListener("click", () => this.tryShowOnClick(event));
  }

  fmt(s, data) {
    data = data
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
    return s.replace("_", data);
  }

  tryShowOnClick(event) {
    // Don't display the context menu if there's a selection.
    // User could be trying to select something and the context menu will undo it.
    if (!window.getSelection().isCollapsed) {
      return;
    }

    if (!event.target.closest("code")) {
      return;
    }

    let tree = document.getElementById("data").getAttribute("data-tree");

    let menuItems = [];

    let index = event.target.closest("[data-i]");
    if (index) {
      index = index.getAttribute("data-i");
      // Comes from the generated page.
      let [jumps, searches] = ANALYSIS_DATA[index];

      for (let { sym, pretty } of jumps) {
        menuItems.push({
          html: this.fmt("Go to definition of _", pretty),
          href: `/${tree}/define?q=${encodeURIComponent(sym)}&redirect=false`,
          icon: "search",
        });
      }

      for (let { sym, pretty } of searches) {
        menuItems.push({
          html: this.fmt("Search for _", pretty),
          href: `/${tree}/search?q=symbol:${encodeURIComponent(
            sym
          )}&redirect=false`,
          icon: "search",
        });
      }
    }

    let word = getTargetWord();
    if (word) {
      // A word was clicked on.
      menuItems.push({
        html: this.fmt("Search for the substring <strong>_</strong>", word),
        href: `/${tree}/search?q=${encodeURIComponent(word)}&redirect=false`,
        icon: "search",
      });
    }

    let token = event.target.closest("[data-symbols]");
    if (token) {
      let symbols = token.getAttribute("data-symbols");
      let visibleToken = token.textContent;
      menuItems.push({
        html: "Sticky highlight",
        href: `javascript:Hover.stickyHighlight('${symbols}', '${visibleToken}')`,
      });
    }

    if (!menuItems.length) {
      return;
    }

    this.menu.innerHTML = nunjucks.render(
      "static/templates/context-menu.html",
      { menuItems }
    );

    let top = event.clientY + window.scrollY;
    let left = event.clientX + window.scrollX;

    this.menu.style.top = top + "px";
    this.menu.style.left = left + "px";

    this.menu.style.display = "";
    this.menu.focus();
  }

  hide() {
    this.menu.style.display = "none";
  }

  get active() {
    return this.menu.style.display != "none";
  }
})();

var Hover = new (class Hover {
  constructor() {
    this.items = [];
    this.sticky = false;
    window.addEventListener("mousedown", () => {
      if (this.sticky) {
        this.deactivate();
      }
    });

    window.addEventListener("mousemove", event => this._handleMouseMove(event));
  }

  _handleMouseMove(event) {
    if (ContextMenu.active || this.sticky) {
      return;
    }

    let symbols = event.target.closest("[data-symbols]");
    if (!symbols) {
      return this.deactivate();
    }

    this.activate(symbols.getAttribute("data-symbols"), symbols.textContent);
  }

  deactivate() {
    for (let item of this.items) {
      item.classList.remove("hovered");
    }
    this.items = [];
    this.sticky = false;
  }

  activate(symbols, visibleToken) {
    this.deactivate();
    this.items = this.findReferences(symbols, visibleToken);
    for (let item of this.items) {
      item.classList.add("hovered");
    }
  }

  findReferences(symbols, visibleToken) {
    function symbolsFromString(symbols) {
      if (!symbols || symbols == "?") {
        // XXX why the `?` special-case?
        return [];
      }
      return symbols.split(",");
    }

    symbols = symbolsFromString(symbols);
    if (!symbols.length) {
      return [];
    }

    symbols = new Set(symbols);

    return [...document.querySelectorAll("span[data-symbols]")].filter(span => {
      // XXX The attribute check is cheaper, probably should be before.
      return (
        span.textContent == visibleToken &&
        symbolsFromString(span.getAttribute("data-symbols")).some(symbol =>
          symbols.has(symbol)
        )
      );
    });
  }

  stickyHighlight(symbols, visibleToken) {
    ContextMenu.hide();
    this.activate(symbols, visibleToken);
    this.sticky = true;
  }
})();

function getTargetWord() {
  var selection = window.getSelection();
  if (!selection.isCollapsed) {
    return null;
  }

  var offset = selection.focusOffset;
  var node = selection.anchorNode;
  var selectedTxtString = node.nodeValue;
  var nonWordCharRE = /[^A-Z0-9_]/i;
  var startIndex =
    selectedTxtString.regexLastIndexOf(nonWordCharRE, offset) + 1;
  var endIndex = selectedTxtString.regexIndexOf(nonWordCharRE, offset);

  // If the regex did not find a start index, start from index 0
  if (startIndex === -1) {
    startIndex = 0;
  }

  // If the regex did not find an end index, end at the position
  // equal to the length of the string.
  if (endIndex === -1) {
    endIndex = selectedTxtString.length;
  }

  // If the offset is beyond the last word, no word was clicked on.
  if (offset > endIndex) {
    return null;
  }

  if (endIndex <= startIndex) {
    return null;
  }

  return selectedTxtString.substr(startIndex, endIndex - startIndex);
}
