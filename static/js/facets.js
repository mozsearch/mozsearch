/**
 * Facets of lists of files (see facet_bar.liquid and `format::explore_facets`
 * in format.rs), as on the /explore/ pages and interdiff summaries.
 *
 * Each facet has values (nested for hierarchical facets, ex: directories),
 * which filter the items (`.facet-items [data-facets]`, whose JSON names the
 * values of each facet each item is in, including their ancestors) to those
 * in any of a facet's selected values, for every facet with selected values.
 * Each value shows how many items it would have given the other facets'
 * selections.  Counts are of distinct paths (`data-path`), since a file can be
 * several items (ex: `/query/`'s definitions and uses of a symbol), and the
 * groups of items (`.facet-group`, which may nest) without any shown items
 * are hidden.  The items can also be grouped by a facet (`data-groups`, whose
 * JSON gives each item's group's sort key and name for each way of grouping),
 * if there's a "Group by" choice; the server groups them by the first.
 *
 * The selections and the grouping go in the URL's query (`facet-KEY=VALUE`,
 * repeated, and `group-by=KEY`), so links reproduce the view.  The box of
 * facets is collapsible, which we remember.
 */
(function () {
  const bar = document.querySelector(".facet-box");
  const container = document.querySelector(".facet-items");
  if (!bar || !container) {
    return;
  }
  const items = [...container.querySelectorAll("[data-facets]")].map(element => ({
    element,
    facets: JSON.parse(element.dataset.facets || "{}"),
    groups: JSON.parse(element.dataset.groups || "{}"),
    path: element.dataset.path || "",
  }));
  const buttons = [...bar.querySelectorAll(".facet-value")].map(button => ({
    button,
    facet: button.closest(".facet").dataset.facet,
    value: button.dataset.value,
    count: button.querySelector(".facet-count"),
  }));
  const groupBy = bar.querySelector(".facet-group-by select");
  const defaultGroupBy = groupBy?.value;
  const status = bar.querySelector(".facet-status");
  const clear = bar.querySelector(".facet-clear");
  const plural = n => `${n} file${n == 1 ? "" : "s"}`;
  const paths = someItems => new Set(someItems.map(item => item.path)).size;
  const allPaths = paths(items);

  // The selected values, by facet.
  const selected = new Map();

  function matches(item, exceptFacet) {
    for (const [facet, values] of selected) {
      if (facet != exceptFacet && values.size &&
          !(item.facets[facet] || []).some(value => values.has(value))) {
        return false;
      }
    }
    return true;
  }

  function update() {
    for (const item of items) {
      item.element.hidden = !matches(item);
    }
    for (const { button, facet, value, count } of buttons) {
      const n = paths(items.filter(item =>
        matches(item, facet) && (item.facets[facet] || []).includes(value)));
      count.textContent = n;
      button.classList.toggle("facet-empty", n == 0);
      button.setAttribute("aria-pressed", selected.get(facet)?.has(value) ? "true" : "false");
    }
    for (const section of container.querySelectorAll(".facet-group")) {
      const shown = new Set([...section.querySelectorAll("[data-facets]")]
        .filter(element => !element.hidden)
        .map(element => element.dataset.path)).size;
      section.hidden = shown == 0;
      const count = section.querySelector(".facet-group-count");
      if (count) {
        count.textContent = `(${plural(shown)})`;
      }
    }
    const shown = paths(items.filter(item => !item.element.hidden));
    const filtering = [...selected.values()].some(values => values.size);
    status.textContent = filtering ? `Showing ${shown} of ${plural(allPaths)}.` : "";
    clear.classList.toggle("facet-inactive", !filtering);
    saveState();
  }

  // Group the items by a way of grouping, sorted by the groups' sort keys and
  // then by path.
  function regroup(key) {
    const groups = new Map();
    for (const item of items) {
      const [sortKey, name] = item.groups[key] || ["", ""];
      if (!groups.has(name)) {
        groups.set(name, { sortKey, name, items: [] });
      }
      groups.get(name).items.push(item);
    }
    const compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
    const sections = [...groups.values()]
      .sort((a, b) => compare(a.sortKey, b.sortKey) || compare(a.name, b.name))
      .map(group => {
        const section = document.createElement("section");
        section.className = "facet-group";
        const header = document.createElement("h3");
        header.className = "facet-group-header";
        const count = document.createElement("span");
        count.className = "facet-group-count";
        header.append(`${group.name} `, count);
        group.items.sort((a, b) => compare(a.path, b.path));
        section.append(header, ...group.items.map(item => item.element));
        return section;
      });
    container.replaceChildren(...sections);
  }

  function saveState() {
    const params = new URLSearchParams(location.search);
    for (const key of [...params.keys()]) {
      if (key.startsWith("facet-") || key == "group-by") {
        params.delete(key);
      }
    }
    for (const [facet, values] of selected) {
      for (const value of values) {
        params.append(`facet-${facet}`, value);
      }
    }
    if (groupBy && groupBy.value != defaultGroupBy) {
      params.set("group-by", groupBy.value);
    }
    const query = params.toString();
    history.replaceState(history.state, "",
      location.pathname + (query ? `?${query}` : "") + location.hash);
  }

  function loadState() {
    const params = new URLSearchParams(location.search);
    for (const [key, value] of params) {
      const facet = key.startsWith("facet-") && key.slice("facet-".length);
      if (facet && buttons.some(b => b.facet == facet && b.value == value)) {
        if (!selected.has(facet)) {
          selected.set(facet, new Set());
        }
        selected.get(facet).add(value);
      }
    }
    const key = params.get("group-by");
    if (groupBy && key && [...groupBy.options].some(option => option.value == key)) {
      groupBy.value = key;
    }
  }

  bar.addEventListener("click", event => {
    const button = event.target.closest(".facet-value");
    if (button) {
      const facet = button.closest(".facet").dataset.facet;
      if (!selected.has(facet)) {
        selected.set(facet, new Set());
      }
      const values = selected.get(facet);
      if (!values.delete(button.dataset.value)) {
        values.add(button.dataset.value);
      }
      update();
    } else if (event.target.closest(".facet-clear")) {
      selected.clear();
      update();
    }
  });
  groupBy?.addEventListener("change", () => {
    regroup(groupBy.value);
    update();
  });

  const toggle = bar.querySelector(".facet-toggle");
  const content = bar.querySelector(".facet-bar");
  function setExpanded(expanded) {
    content.hidden = !expanded;
    toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
    toggle.querySelector(".facet-toggle-icon").classList.toggle("expanded", expanded);
  }
  toggle.addEventListener("click", () => {
    setExpanded(content.hidden);
    localStorage.setItem("facets-collapsed", content.hidden ? "1" : "0");
  });
  setExpanded(localStorage.getItem("facets-collapsed") != "1");

  loadState();
  if (groupBy && groupBy.value != defaultGroupBy) {
    regroup(groupBy.value);
  }
  update();
})();
