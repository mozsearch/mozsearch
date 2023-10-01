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

  fmtLang(lang) {
      lang = lang.toUpperCase();
      if (lang === "CPP") {
        lang = "C++";
      }
      return lang;
  }

  tryShowOnClick(event) {
    // Don't display the context menu if there's a selection.
    // User could be trying to select something and the context menu will undo it.
    if (!window.getSelection().isCollapsed) {
      return;
    }

    // We expect to find symbols in:
    // - source listings ("code")
    // - diagrams ("svg")
    // - breadcrumbs ("breadcrumbs")
    if (!event.target.closest("code") && !event.target.closest("svg") && !event.target.closest(".breadcrumbs")) {
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

      // ## First pass: Process symbols and potentially filter out implicit constructors
      //
      // In the future we can potentially use this pass to do more clever things,
      // but right now the main interesting situation that can arise is that the
      // user is clicking on a constructor where we have both the constructor
      // symbol plus all of the implicit constructors that will be invoked as
      // part of the constructor and we are weirdly attributing to the constructor.
      //
      // We can detect this case because we can detect when the user is clicking
      // on a line that's already the target of a definition jump.  And then in
      // that case we can filter out all the symbols that aren't definition jumps.
      //
      // In general, we only expect to see multiple symbols here when the symbol
      // varies per platform or as a result of implicit constructors like this.
      // Our logic to remove implicit constructors here will not affect the
      // platform case because all symbols will have the line as a definition.
      // (For other platforms where the definition is on a different line, the
      // symbol won't be present here because it won't have been mered in by the
      // merge-analyses step.)
      let filteredSymPairs = [];
      let sawDef = false;
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

        // The symInfo is self-identifying via `pretty` and `sym` so we don't
        // need to try and include any extra context.
        exposeSymbolsForDebugging.push(symInfo);

        if (symInfo?.jumps?.idl === sourceLineClicked ||
            symInfo?.jumps?.def === sourceLineClicked ) {
          if (!sawDef) {
            // Transition to "kick out the implicit constructors" mode.
            sawDef = true;
            filteredSymPairs = [];
          }
          filteredSymPairs.push([sym, symInfo]);
        } else if (!sawDef) {
          filteredSymPairs.push([sym, symInfo]);
        }
      }

      for (const [sym, symInfo] of filteredSymPairs) {
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

        // Define a helper we can also use for the binding slots below.
        const jumpify = (jumpref, pretty) => {
          if (!jumpref.jumps) {
            return;
          }
          if (jumpref.jumps.idl && jumpref.jumps.idl !== sourceLineClicked) {
            jumpMenuItems.push({
              html: this.fmt("Go to IDL definition of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.idl}`,
              icon: "export-alt",
              section: "jumps",
            });
          }

          if (jumpref.jumps.def && jumpref.jumps.def !== sourceLineClicked) {
            jumpMenuItems.push({
              html: this.fmt("Go to definition of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.def}`,
              icon: "export-alt",
              section: "jumps",
            });
          }

          if (jumpref.jumps.decl && jumpref.jumps.decl !== sourceLineClicked) {
            jumpMenuItems.push({
              html: this.fmt("Go to declaration of <strong>_</strong>", pretty),
              href: `/${tree}/source/${jumpref.jumps.decl}`,
              icon: "export",
              section: "jumps",
            });
          }
        }

        let searches = [];

        // If we have a slotOwner, it can help make our "go to def" description
        // more descriptive and identical to what would be generated for when
        // the bindingSlot that refers to us from our slotOwner would describe.
        if (symInfo.meta?.slotOwner) {
          let slotOwner = symInfo.meta.slotOwner;
          let ownerJumpref = SYM_INFO[slotOwner.sym];
          // XXX Ignore no_crossref data that's currently not useful/used.
          if (ownerJumpref && ownerJumpref.sym && ownerJumpref.pretty) {
            let implKind = ownerJumpref.meta.implKind || "impl";
            if (implKind === "idl") {
              implKind = "IDL";
              diagrammableSyms = [];
            }

            let maybeLang = "";
            if (slotOwner.slotLang) {
              maybeLang = ` ${this.fmtLang(slotOwner.slotLang)}`;
            }

            const canonLabel = `${implKind}${maybeLang} ${slotOwner.slotKind} ${symInfo.pretty}`;
            jumpify(symInfo, canonLabel);
            searches.push([ canonLabel, sym ])
            jumpify(ownerJumpref, ownerJumpref.pretty);
          } else {
            jumpify(symInfo, symInfo.pretty);
            searches.push([ symInfo.pretty, sym ]);
          }
        } else {
          jumpify(symInfo, symInfo.pretty);
          searches.push([ symInfo.pretty, sym ]);
        }

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
              const lang = slot.slotLang;
              if (lang === "cpp") {
                diagrammableSyms.push(slotJumpref);
              }
              maybeLang = ` ${this.fmtLang(lang)}`;
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
            html: this.fmt("Search for <strong>_</strong>", search[0]),
            href: `/${tree}/search?q=symbol:${encodeURIComponent(
              search[1]
            )}&redirect=false`,
            icon: "search",
            section: "symbol-searches",
          });
        }

        if (Settings.diagramming.enabled) {
          for (const jumpref of diagrammableSyms) {
            // Always offer to diagram uses of things
            let queryString = `calls-to:'${jumpref.pretty}' depth:4`;
            //const queryString = `calls-to-sym:'${jumpref.sym}' depth:4`;
            extraMenuItems.push({
              html: this.fmt("Uses diagram of <strong>_</strong>", jumpref.pretty),
              href: `/${tree}/query/default?q=${encodeURIComponent(queryString)}`,
              icon: "brush",
              section: "diagrams",
            });

            // Always offer to diagram uses of things
            queryString = `calls-from:'${jumpref.pretty}' depth:4`;
            //const queryString = `calls-to-sym:'${jumpref.sym}' depth:4`;
            extraMenuItems.push({
              html: this.fmt("Calls diagram of <strong>_</strong>", jumpref.pretty),
              href: `/${tree}/query/default?q=${encodeURIComponent(queryString)}`,
              icon: "brush",
              section: "diagrams",
            });

            // Offer class diagrams for classes
            if (jumpref?.meta?.kind === "class") {
              queryString = `class-diagram:'${jumpref.pretty}' depth:4`;
              //const queryString = `calls-to-sym:'${jumpref.sym}' depth:4`;
              extraMenuItems.push({
                html: this.fmt("Class diagram of <strong>_</strong>", jumpref.pretty),
                href: `/${tree}/query/default?q=${encodeURIComponent(queryString)}`,
                icon: "brush",
                section: "diagrams",
              });
            }

            // Offer inheritance diagrams for methods that are involved in an
            // override hierarchy.  This does not currently work for classes
            // despite the name demanding it.  (cmd_traverse would like a minor
            // cleanup.)
            if (jumpref?.meta?.kind === "method" &&
                (jumpref?.meta?.overrides?.length || jumpref?.meta?.overriddenBy?.length)) {
              queryString = `inheritance-diagram:'${jumpref.pretty}' depth:4`;
              //const queryString = `calls-to-sym:'${jumpref.sym}' depth:4`;
              extraMenuItems.push({
                html: this.fmt("Inheritance diagram of <strong>_</strong>", jumpref.pretty),
                href: `/${tree}/query/default?q=${encodeURIComponent(queryString)}`,
                icon: "brush",
                section: "diagrams",
              });
            }
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
        icon: "font",
        section: "text-searches",
      });
    }

    if (symbolToken) {
      let symbols = symbolToken.getAttribute("data-symbols");
      let visibleToken = symbolToken.textContent;
      menuItems.push({
        html: "Sticky highlight",
        href: `javascript:Hover.stickyHighlight('${symbols}', '${visibleToken}')`,
        icon: "tasks",
        section: "highlights",
      });
    }

    menuItems.push(...extraMenuItems);

    if (!menuItems.length) {
      return;
    }

    let suppression = new Set();
    this.menu.innerHTML = "";
    let lastSection = null;
    for (let item of menuItems) {
      // Avoid adding anything we've definitely added before.  This currently
      // can happen for hierarchical diagrams where we unify based on the
      // "pretty" and in particular for IDL interfaces/methods where the iface
      // pretties will be the same as the C++ impl pretty.
      let itemAsJson = JSON.stringify(item);
      if (suppression.has(itemAsJson)) {
        continue;
      }
      suppression.add(itemAsJson);

      let li = document.createElement("li");
      li.classList.add("contextmenu-row")
      if (lastSection === null) {
        lastSection = item.section;
      } else if (lastSection === item.section) {
        // nothing to do for the same section
        li.classList.add("contextmenu-same-section");
      } else {
        li.classList.add("contextmenu-new-section");
        lastSection = item.section;
      }

      let link = li.appendChild(document.createElement("a"));
      link.href = item.href;

      link.classList.add("contextmenu-link");
      if (item.icon) {
        link.classList.add(`icon-${item.icon}`);
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
    this.graphItems = [];
    this.hoveredElem = null;
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

    let elem = event.target?.closest("[data-symbols]");
    // Don't recompute things if we're still hovering over the same element.
    if (elem === this.hoveredElem) {
      return;
    }
    if (!elem) {
      return this.deactivate();
    }

    let symbolNames = this.symbolsFromString(elem.getAttribute("data-symbols"));
    // We're hovering over a graph so we also want to hover related graph nodes.
    // We will still also potentially want to highlight any document spans as
    // well.
    if (elem.tagName === "g") {
      this.activateDiagram(symbolNames);
    }

    this.activate(symbolNames, elem.textContent);
    this.hoveredElem = elem;
  }

  symbolsFromString(symbolStr) {
    if (!symbolStr || symbolStr == "?") {
      // XXX why the `?` special-case?
      return [];
    }
    return symbolStr.split(",");
  }

  deactivate() {
    for (let item of this.items) {
      item.classList.remove("hovered");
    }
    this.hoveredElem = null;
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

  findReferences(symbolNames, visibleToken) {
    if (!symbolNames.length) {
      return [];
    }

    let symbols = new Set(symbolNames);

    return [...document.querySelectorAll("span[data-symbols]")].filter(span => {
      // XXX The attribute check is cheaper, probably should be before.
      return (
        span.textContent == visibleToken &&
        this.symbolsFromString(span.getAttribute("data-symbols")).some(symbol =>
          symbols.has(symbol)
        )
      );
    });
  }

  activateDiagram(symbolNames) {

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
