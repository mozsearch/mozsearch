"use strict";

// The crash view's breadcrumbs and thread picker (see tools/src/crash_stack.rs's
// `crumbs_html`): a crash report's view's breadcrumbs move into the page's
// (after its tree's), the report's ID with its menu, and the thread (the
// crashing one's, or with every thread's, the one scrolled to), with a picker
// of the threads, their kinds as its headings.  (With every thread's, the
// navigation panel's facets filter the threads; see facets.js and
// `thread_facets`.)

// A breadcrumb's menu (as the tree switcher's), below its breadcrumb
// (`anchor`): `fill` fills it when it opens, and gives the item to focus,
// if not the first.
class CrashMenu extends ContextMenuBase {
  constructor(button, menu, anchor, fill) {
    super();
    this.button = button;
    this.menu = menu;
    this.anchor = anchor;
    this.fill = fill;
    if (!button || !menu) {
      return;
    }
    button.addEventListener("click", () => {
      if (this.isShown()) {
        this.hide();
      } else {
        this.open();
      }
    });
  }

  open() {
    const focus = this.fill(this);
    this.menu.style.display = "flex";
    this.button.setAttribute("aria-expanded", "true");
    // (Within the window.)
    const left = Math.min(this.anchor?.offsetLeft ?? 0,
                          document.documentElement.clientWidth - this.menu.offsetWidth - 16);
    this.menu.style.left = Math.max(0, left) + "px";
    if (focus) {
      this.focusElement(focus);
    } else if (this.columns.length && this.columns[0].length) {
      this.focusItemAt({ col: 0, row: 0 }, "first");
    }
  }

  isShown() {
    return this.menu?.style.display == "flex";
  }

  hide() {
    super.hide();
    this.button?.setAttribute("aria-expanded", "false");
  }

  hideOnMouseDown(event) {
    if (this.menu && (this.menu.contains(event.target) || this.button.contains(event.target))) {
      return;
    }
    this.hide();
  }
}

var CrashView = new (class CrashView {
  constructor() {
    this.crumbs = document.querySelector(".crash-crumbs");
    if (!this.crumbs) {
      return;
    }
    const breadcrumbs = document.querySelector(".breadcrumbs");
    if (breadcrumbs) {
      breadcrumbs.append(this.crumbs);
    }
    this.crumbs.hidden = false;
    this.breadcrumbs = breadcrumbs;

    const list = document.getElementById("crash-thread-list");
    this.threads = list ? JSON.parse(list.textContent) : [];
    this.sections = [...document.querySelectorAll(".crash-view .crash-thread")];
    this.threadLink = this.crumbs.querySelector(".crash-crumb-thread-link");

    this.reportMenu = new CrashMenu(
      document.getElementById("crash-report-menu-button"),
      document.getElementById("crash-report-menu"),
      this.crumbs.querySelector(".crash-crumb-report"),
      menu => this.fillReportMenu(menu));
    // (The report's ID's context menu is its menu.)
    this.crumbs.querySelector(".crash-crumb-report")?.addEventListener("contextmenu", event => {
      event.preventDefault();
      this.reportMenu.open();
    });
    const picker = document.getElementById("crash-thread-picker");
    if (picker) {
      this.threadMenu = new CrashMenu(
        picker,
        document.getElementById("crash-thread-menu"),
        this.crumbs.querySelector(".crash-crumb-thread"),
        menu => this.fillThreadMenu(menu));
    }

    // (The thread scrolled to, with every thread's.)
    if (this.sections.length > 1) {
      let pending = false;
      document.addEventListener("scroll", () => {
        if (pending) {
          return;
        }
        pending = true;
        requestAnimationFrame(() => {
          pending = false;
          this.updateCurrentThread();
        });
      }, { capture: true, passive: true });
      // (Until the page's scrolled some other way than by picking a thread.)
      for (const type of ["wheel", "touchmove", "keydown"]) {
        document.addEventListener(type, () => {
          this.picked = null;
        }, { capture: true, passive: true });
      }
      this.updateCurrentThread();
    }
  }

  // The thread whose section is at the top of the page, under its sticky
  // header and breadcrumbs (or just below them, as scrolling to it leaves
  // it); or the thread picked, while it's in view below it (ex: near the
  // page's end, which can't be scrolled to the top).
  updateCurrentThread() {
    const top = (this.breadcrumbs ? this.breadcrumbs.getBoundingClientRect().bottom : 0) + 40;
    let current = null;
    for (const section of this.sections) {
      if (section.hidden) {
        continue;
      }
      if (!current || section.getBoundingClientRect().top <= top) {
        current = section;
      } else {
        break;
      }
    }
    if (this.picked) {
      const pickedTop = this.picked.getBoundingClientRect().top;
      if (!this.picked.hidden && pickedTop > top && pickedTop < window.innerHeight) {
        current = this.picked;
      } else {
        this.picked = null;
      }
    }
    if (current && this.threadLink) {
      this.threadLink.textContent = current.dataset.label;
      this.threadLink.setAttribute("href", "#" + current.id);
    }
  }

  fillReportMenu(menu) {
    const data = this.crumbs.dataset;
    const items = [
      new MenuItem({ html: "Open the report on crash-stats", href: data.reportUrl, icon: "export-alt" }),
      new MenuItem({
        html: "Copy the report's ID",
        icon: "docs",
        action: () => {
          navigator.clipboard?.writeText(data.report);
          menu.hide();
        },
      }),
    ];
    if (data.signatureUrl) {
      items.push(new MenuItem({ html: "Its signature's stack", href: data.signatureUrl, icon: "search" }));
    }
    if (data.analyzeUrl) {
      items.push(new MenuItem({ html: "Analyze its signature's recent crash reports", href: data.analyzeUrl, icon: "tasks" }));
    }
    if (data.threadsUrl) {
      items.push(new MenuItem({ html: escapeHtml(data.threadsLabel), href: data.threadsUrl, icon: "search" }));
    }
    fillCrashMenu(menu, [[{ items }]]);
  }

  // The threads by their kinds (those with the threads saying the most
  // first: the crashed one's, then those with notable, active, waiting,
  // unknown and idle threads), in columns; the current thread's focused.
  fillThreadMenu(menu) {
    const RANKS = { crashed: 0, active: 2, waiting: 3, unknown: 4, idle: 5 };
    const rank = thread => RANKS[thread.stateSlug] ?? 1;
    const groups = new Map();
    for (const thread of this.threads) {
      if (!groups.has(thread.kind)) {
        groups.set(thread.kind, { label: thread.kind, detail: thread.owner, threads: [] });
      }
      groups.get(thread.kind).threads.push(thread);
    }
    const sorted = [...groups.values()];
    for (const group of sorted) {
      group.threads.sort((a, b) => rank(a) - rank(b) || a.index - b.index);
      group.rank = rank(group.threads[0]);
    }
    sorted.sort((a, b) => a.rank - b.rank || b.threads.length - a.threads.length ||
                          a.label.localeCompare(b.label));

    for (const group of sorted) {
      group.items = group.threads.map(thread => new MenuItem({
        html: `<span class="crash-menu-thread">${escapeHtml(thread.label)}</span> ` +
              `<span class="crash-thread-state crash-state-${escapeHtml(thread.stateSlug)}">${escapeHtml(thread.state)}</span>`,
        classNames: ["indent"],
        attrs: { "data-thread": String(thread.index) },
        action: () => {
          menu.hide();
          this.showThread(thread.index);
        },
      }));
    }

    // (About as many rows in each column, at most 4 columns.)
    const rows = sorted.reduce((n, group) => n + group.items.length + 1, 0);
    const count = Math.min(4, Math.max(1, Math.ceil(rows / 14)));
    const perColumn = Math.ceil(rows / count);
    const columns = [[]];
    let inColumn = 0;
    for (const group of sorted) {
      if (inColumn > 0 && inColumn + group.items.length + 1 > perColumn && columns.length < count) {
        columns.push([]);
        inColumn = 0;
      }
      columns[columns.length - 1].push(group);
      inColumn += group.items.length + 1;
    }
    fillCrashMenu(menu, columns);
    const current = this.threadLink?.getAttribute("href")?.replace(/^#thread-/, "");
    return menu.menu.querySelector(`a[data-thread="${current}"]`);
  }

  // Scrolls to thread `index`'s section (shown, if the facets hid it).
  showThread(index) {
    const section = document.getElementById(`thread-${index}`);
    if (!section) {
      return;
    }
    section.hidden = false;
    section.scrollIntoView();
    this.picked = section;
    history.replaceState(history.state, "", `#thread-${index}`);
    this.updateCurrentThread();
  }
})();

function escapeHtml(text) {
  return String(text).replace(/[&<>"']/g, c => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}

// Fills `menu` with `columns` of groups (`label` and `detail`, if they have
// them, and their `items`), as the tree switcher's.
function fillCrashMenu(menu, columns) {
  const boxes = [];
  menu.columns = [];
  for (const groups of columns) {
    const box = document.createElement("div");
    const column = [];
    for (const group of groups) {
      if (group.label) {
        const label = document.createElement("div");
        label.classList.add("context-menu-group-label");
        label.textContent = group.label;
        if (group.detail) {
          const detail = document.createElement("span");
          detail.classList.add("crash-menu-owner");
          detail.textContent = group.detail;
          label.append(" ", detail);
        }
        box.append(label);
        column.push({ isFocusable: () => false });
      }
      const list = document.createElement("ul");
      list.setAttribute("role", "group");
      for (const item of group.items) {
        const li = item.createListItem(menu, { col: menu.columns.length, row: column.length });
        list.append(li);
        column.push(item);
      }
      box.append(list);
    }
    menu.columns.push(column);
    boxes.push(box);
  }
  menu.menu.replaceChildren(...boxes);
}
