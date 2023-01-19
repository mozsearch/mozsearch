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

    if (!event.target.closest("code") && !event.target.closest("svg")) {
      return;
    }

    let tree = document.getElementById("data").getAttribute("data-tree");

    // Figure out the source line this click was on, if it was on any line, so
    // that we can compare it against jump strings in order to avoid offering
    // the option to jump to the line the user literally just clicked on.
    let sourceLineClicked = null;
    {
      let sourceLineNode = event.target.closest("code");
      let lineNumberNode = sourceLineNode?.previousElementSibling;
      if (lineNumberNode && Router.sourcePath) {
        sourceLineClicked = `${Router.sourcePath}#${lineNumberNode.dataset.lineNumber}`;
      }
    }

    // jumps come first
    let jumpMenuItems = [];
    // then searches
    let searchMenuItems = [];
    // the the text search and sticky highlight option
    // then these extra menu items which are for new/experimental features where
    // we don't want to mess with muscle memory at the top of the list.
    let extraMenuItems = [];

    let symbolToken = event.target.closest("[data-symbols]");
    if (symbolToken) {
      let symbols = symbolToken.getAttribute("data-symbols").split(",");

      const seenSyms = new Set();
      // For debugging/investigation purposes, expose the symbols that got
      // clicked on on the window global.
      const exposeSymbolsForDebugging = window.CLICKED_SYMBOLS = [];
      for (const sym of symbols) {
        // Avoid processing the same symbol more than once.
        if (seenSyms.has(sym)) {
          continue;
        }

        const symInfo = SYM_INFO[sym];
        // XXX Ignore no_crossref data that's currently not useful/used.
        if (!symInfo || !symInfo.sym || !symInfo.pretty) {
          continue;
        }

        // TODO: Revisit this as the diagramming mechanism better understands how
        // to deal with slots.  There are some complications related to this
        // because currently the JS XPIDL binding situation is such that there are
        // way too many false-positives so it's usually going to be bad news to
        // try and traverse the JS binding edges, so we sorta don't want the
        // traversal to even try yet.
        //
        // IDL symbols are usually not directly diagrammable, so for now if we see
        // we're an IDL symbol and we have binding slots, we instead will just use
        // the C++ binding symbols.
        let diagrammableSyms = [symInfo];

        // The symInfo is self-identifying via `pretty` and `sym` so we don't
        // need to try and include any extra context.
        exposeSymbolsForDebugging.push(symInfo);

        // Define a helper we can also use for the binding slots below.
        const jumpify = (jumpref, pretty) => {
          if (!jumpref.jumps) {
            return;
          }
          if (jumpref.jumps.idl && jumpref.jumps.idl !== sourceLineClicked) {
            jumpMenuItems.push({
              html: this.fmt("Go to IDL definition of _", pretty),
              href: `/${tree}/source/${jumpref.jumps.idl}`,
              icon: "search",
            });
          }

          if (jumpref.jumps.def && jumpref.jumps.def !== sourceLineClicked) {
            jumpMenuItems.push({
              html: this.fmt("Go to definition of _", pretty),
              href: `/${tree}/source/${jumpref.jumps.def}`,
              icon: "search",
            });
          }

          if (jumpref.jumps.decl && jumpref.jumps.decl !== sourceLineClicked) {
            jumpMenuItems.push({
              html: this.fmt("Go to declaration of _", pretty),
              href: `/${tree}/source/${jumpref.jumps.decl}`,
              icon: "search",
            });
          }
        }

        jumpify(symInfo, symInfo.pretty);

        // Slot owner
        if (symInfo.meta?.slotOwner) {
          let slotOwner = symInfo.meta.slotOwner;
          let ownerJumpref = SYM_INFO[slotOwner.sym];
          // XXX Ignore no_crossref data that's currently not useful/used.
          if (ownerJumpref && ownerJumpref.sym && ownerJumpref.pretty) {
            jumpify(ownerJumpref, ownerJumpref.pretty);
          }
        }

        let searches = [[ symInfo.pretty, sym ]];
        // Binding slots
        if (symInfo.meta?.bindingSlots) {
          let implKind = symInfo.meta.implKind || "impl";
          if (implKind === "idl") {
            implKind = "IDL";
            diagrammableSyms = [];
          }

          let allSearchSyms = [];
          for (const slot of symInfo.meta.bindingSlots) {
            // XXX Ignore no_crossref data that's currently not useful/used.
            let slotJumpref = SYM_INFO[slot.sym];
            // (we do handle the pretty not existing below)
            if (!slotJumpref || !slotJumpref.sym) {
              continue;
            }

            let maybeLang = "";
            if (slot.slotLang) {
              let lang = slot.slotLang.toUpperCase();
              if (lang === "CPP") {
                lang = "C++";
                diagrammableSyms.push(slotJumpref);
              }
              maybeLang = ` ${lang}`;
            }

            // Favor the slot's pretty if available.
            const effectivePretty = slotJumpref?.pretty || symInfo.pretty;
            let slotPretty =
              `${implKind}${maybeLang} ${slot.slotKind} ${effectivePretty}`;
            searches.push([slotPretty, slot.sym]);
            allSearchSyms.push(slot.sym);

            if (slotJumpref) {
              jumpify(slotJumpref, slotPretty);
            }
          }

          // If there were multiple language bindings that we think might exist,
          // then generate a single roll-up search.
          if (allSearchSyms.length > 1) {
            // Eat the default search if this was IDL, as currently the "search"
            // endpoint search for the synthetic symbol will only do upsells
            // which is not what people are used to.
            if (implKind === "IDL") {
              searches.shift();
              // Do put the synthetic symbol at the start of the explicit symbol
              // list, however.
              allSearchSyms.unshift(sym);
            }
            searches.push([`${implKind} ${symInfo.meta.kind} ${symInfo.pretty}`, allSearchSyms.join(",")]);
          }
        }

        for (const search of searches) {
          searchMenuItems.push({
            html: this.fmt("Search for _", search[0]),
            href: `/${tree}/search?q=symbol:${encodeURIComponent(
              search[1]
            )}&redirect=false`,
            icon: "search",
          });
        }

        if (Settings.diagramming.enabled) {
          for (const jumpref of diagrammableSyms) {
            const queryString = `calls-to:'${jumpref.pretty}' depth:4`;
            extraMenuItems.push({
              html: this.fmt("Diagram uses of _", jumpref.pretty),
              href: `/${tree}/query/default?q=${encodeURIComponent(queryString)}`,
              icon: "search"
            });
          }
        }
      }
    }

    let menuItems = jumpMenuItems.concat(searchMenuItems);

    let word = getTargetWord();
    if (word) {
      // A word was clicked on.
      menuItems.push({
        html: this.fmt("Search for the substring <strong>_</strong>", word),
        href: `/${tree}/search?q=${encodeURIComponent(word)}&redirect=false`,
        icon: "search",
      });
    }

    if (symbolToken) {
      let symbols = symbolToken.getAttribute("data-symbols");
      let visibleToken = symbolToken.textContent;
      menuItems.push({
        html: "Sticky highlight",
        href: `javascript:Hover.stickyHighlight('${symbols}', '${visibleToken}')`,
      });
    }

    menuItems.push(...extraMenuItems);

    if (!menuItems.length) {
      return;
    }

    this.menu.innerHTML = "";
    for (let item of menuItems) {
      let li = document.createElement("li");
      let link = li.appendChild(document.createElement("a"));
      link.href = item.href;
      link.classList.add("mimetype-fixed-container");
      // Cancel out the default "unknown" icon we get from the above so there's
      // no icon displayed (by default but also in conjunction with the below).
      link.classList.add("mimetype-no-icon");
      // So for a long time we would display a search icon in the context menu,
      // but only ever a search icon because that's the only thing we hardcode
      // in the menuItems we would push in the logic above.
      //
      // But we accidentally removed the icon in
      // 6c35b409a1d4dde7581a59bbad317a472732c15c in late 2021, so we haven't
      // had the icon around anymore, so the context menu has had the whitespace
      // where an icon would go, but we wouldn't display any.  The other icons
      // removed at the same time were intended for the context menu to
      // differentiate between searches, jumps, etc.
      //
      // In order to maintain the existing status quo, I'm commenting out the
      // logic below so we don't have an icon, but we should probably strongly
      // consider bringing icons back in a way that provides for the ability to
      // visually distinguish stuff.
      /*
      if (item.icon) {
        link.classList.add(item.icon);
      }
      */
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
        this.menu.style.bottom = viewportHeight - y + "px";
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

    let symbols = event.target?.closest("[data-symbols]");
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
  let string = node?.nodeValue;

  if (!string?.length) {
    return null;
  }

  function isWordChar(character) {
    // TODO: this could be more non-ascii friendly.
    //
    // Notable Changes:
    // - We have added "#" to deal with JS private symbols.  This will widen
    //   C preprocessor directives to include the leading #, which makes sense.
    //   This will also impact use of the "stringizing operator" for macros,
    //   where it won't be what we want.
    return /[#A-Z0-9_]/i.test(character);
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
