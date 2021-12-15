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
    window.addEventListener("click", event => this.tryShowOnClick(event));
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

    this.menu.innerHTML = "";
    for (let item of menuItems) {
      let li = document.createElement("li");
      let link = li.appendChild(document.createElement("a"));
      link.href = item.href;
      link.classList.add("icon");
      if (item.icon) {
        link.classList.add(item.icon);
      }
      link.innerHTML = item.html;
      this.menu.appendChild(li);
    }

    let x = event.clientX + window.scrollX;
    let y = event.clientY + window.scrollY;

    let viewportHeight = window.innerHeight;
    let spaceTowardsBottom = viewportHeight - event.clientY;
    let spaceTowardsTop = viewportHeight - spaceTowardsBottom;

    // Position the menu towards the bottom, and if that overflows and there's
    // more space to the top, flip it.
    this.menu.classList.remove("bottom");
    this.menu.style.bottom = "";
    this.menu.style.top = y + "px";
    this.menu.style.left = x + "px";
    this.menu.style.maxHeight = "none";

    this.menu.style.display = "";
    this.menu.style.opacity = "0";

    let rect = this.menu.getBoundingClientRect();
    // If it overflows, either flip it or constrain its height.
    if (rect.height > spaceTowardsBottom) {
      if (spaceTowardsTop > spaceTowardsBottom) {
        // Position it towards the top.
        this.menu.classList.add("bottom");
        this.menu.style.bottom = (viewportHeight - y) + "px";
        this.menu.style.top = "";
        if (rect.height > spaceTowardsTop) {
          this.menu.style.maxHeight = spaceTowardsTop + "px";
        }
      } else {
        // Constrain its height.
        this.menu.style.maxHeight = spaceTowardsBottom + "px";
      }
    }

    // Now the menu is correctly positioned, show it.
    this.menu.style.opacity = "";
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
  let selection = window.getSelection();
  if (!selection.isCollapsed) {
    return null;
  }

  let offset = selection.focusOffset;
  let node = selection.anchorNode;
  let string = node.nodeValue;

  if (!string.length) {
    return null;
  }

  function isWordChar(character) {
    // TODO: this could be more non-ascii friendly.
    return /[A-Z0-9_]/i.test(character);
  }

  if (offset < string.length && !isWordChar(string[offset])) {
    // Not really in a word.
    return null;
  }

  let start = offset;
  let end = offset;

  while (start > 0 && isWordChar(string[start - 1])) {
    --start;
  }
  while (end < string.length && isWordChar(string[end])) {
    ++end;
  }

  if (end <= start) {
    return null;
  }

  return string.substring(start, end);
}
