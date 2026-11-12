/**
 * Facets of lists of files (see facet_bar.liquid and `format::explore_facets`
 * in format.rs), as on the /explore/ pages and interdiff summaries, or of
 * other items (ex: a spec repository's pull requests; see
 * `format::format_spec_prs`).
 *
 * Each facet has values (nested for hierarchical facets, ex: directories),
 * which filter the items (`.facet-items [data-facets]`, whose JSON names the
 * values of each facet each item is in, including their ancestors, and which
 * can be in other items, ex: `/query/`'s lines in their files) to those
 * in any of a facet's selected values, for every facet with selected values.
 * Each value shows how many items it would have given the other facets'
 * selections.  Counts are of distinct paths (`data-path`), since a file can be
 * several items (ex: `/query/`'s definitions and uses of a symbol), and the
 * groups of items (`.facet-group`, which may nest) without any shown items
 * are hidden.  The items can also be grouped by a facet (`data-groups`, whose
 * JSON gives each item's group's sort key and name for each way of grouping),
 * if there's a "Group by" choice; the server groups them by the first.  And
 * sorted by their changes' sizes (`data-size`), if there's a "Sort by"
 * choice, rather than by path (see `fileOrderKey`).  The groups' headers'
 * context menu has the choices too.
 *
 * A value can be left out instead (shift-click; see `toggleValue`): the items in
 * any of a facet's left out values are hidden (ex: a subsystem, and so its
 * nested subsystems), as well as those not in its selected values, if it
 * has any.
 *
 * The items can stand in for others, ex: a diagram's nodes (see
 * query_results/graph_input.liquid), whose page then shows only those of the
 * shown items, on their container's "facetschange" events (see graph.js).
 * And the items in an element with facet values for all of them
 * (`data-facet-scope`, ex: `/query/`'s groups of results of a relation to the
 * queried symbols; see query_results/kind_group.liquid) have those too.
 *
 * A facet's values can be checkboxes (`data-toggles`, ex: `/query/`'s
 * results' relations), whose clicks show or leave out their items, rather
 * than showing only them; and a facet can leave out some values at first
 * (`data-default-excluded`), unless the URL says otherwise for it (with an
 * empty `-facet-KEY=` for none).  Other scripts can show and leave out a
 * facet's values too (`window.ResultFacets`, ex: the overview of the results'
 * symbols; see query-overview.js).
 *
 * The selections, the left out values, the grouping and the sorting go in
 * the URL's query (`facet-KEY=VALUE` and `-facet-KEY=VALUE`, as a query's
 * negated terms are, repeated, `group-by=KEY` and `sort-by=size`), so links
 * reproduce the view.  The box of facets is collapsible, which we remember.
 *
 * Values with nested values have a button showing and hiding them (with
 * Shift, theirs too), and levels of values with elided ones (see
 * `FacetView::new` in facets.rs), a button showing those; which show at first
 * is the server's choice.  Hierarchical facets' labels have buttons showing
 * all of their values, or only the biggest at the top.  Selected values
 * always show.
 */
(function () {
  const bar = document.querySelector(".facet-box");
  const container = document.querySelector(".facet-items");
  if (!bar || !container) {
    return;
  }
  // Items can be in others (ex: `/query/`'s lines in their files), whose facets
  // and path they have too, unless they say otherwise.
  const items = [...container.querySelectorAll("[data-facets]")].map(element => {
    const outer = element.parentElement.closest("[data-facets]");
    const scope = element.closest("[data-facet-scope]");
    return {
      element,
      facets: {
        ...JSON.parse(scope?.dataset.facetScope || "{}"),
        ...JSON.parse(outer?.dataset.facets || "{}"),
        ...JSON.parse(element.dataset.facets || "{}"),
      },
      groups: JSON.parse(element.dataset.groups || "{}"),
      path: element.dataset.path || outer?.dataset.path || "",
      size: Number(element.dataset.size || 0),
    };
  });
  const buttons = [...bar.querySelectorAll(".facet-value")].map(button => ({
    button,
    facet: button.closest(".facet").dataset.facet,
    value: button.dataset.value,
    count: button.querySelector(".facet-count"),
    title: button.title,
  }));
  // The facets of checkboxes, and the values each leaves out at first.
  const toggles = new Set([...bar.querySelectorAll(".facet[data-toggles]")]
    .map(facet => facet.dataset.facet));
  const defaultExcluded = new Map([...bar.querySelectorAll(".facet[data-default-excluded]")]
    .map(facet => [facet.dataset.facet, new Set(facet.dataset.defaultExcluded.split(" "))]));
  const groupBy = bar.querySelector(".facet-group-by select");
  const defaultGroupBy = groupBy?.value;
  const sortBy = bar.querySelector(".facet-sort-by select");
  const defaultSortBy = sortBy?.value;
  const status = bar.querySelector(".facet-status");
  const clear = bar.querySelector(".facet-clear");
  // (The items are files, unless the bar says otherwise.)
  const noun = bar.dataset.noun || "file";
  const nouns = bar.dataset.nouns || "files";
  const plural = n => `${n} ${n == 1 ? noun : nouns}`;
  const paths = someItems => new Set(someItems.map(item => item.path)).size;
  const allPaths = paths(items);

  // The selected values, and the left out ones, by facet.
  const selected = new Map();
  const excluded = new Map();
  const valuesOf = (map, facet) => {
    if (!map.has(facet)) {
      map.set(facet, new Set());
    }
    return map.get(facet);
  };

  function matches(item, exceptFacet) {
    for (const [facet, values] of selected) {
      if (facet != exceptFacet && values.size &&
          !(item.facets[facet] || []).some(value => values.has(value))) {
        return false;
      }
    }
    for (const [facet, values] of excluded) {
      if (facet != exceptFacet && (item.facets[facet] || []).some(value => values.has(value))) {
        return false;
      }
    }
    return true;
  }

  // A value's click: selecting it, or unselecting it if it's selected (or
  // selecting it if it's left out); or with Shift, leaving it out, or not if
  // it's left out (and leaving it out if it's selected).
  function toggleValue(facet, value, shift) {
    const included = valuesOf(selected, facet);
    const left = valuesOf(excluded, facet);
    // (A checkbox's: shown or left out.)
    if (toggles.has(facet)) {
      if (!left.delete(value)) {
        left.add(value);
      }
      return;
    }
    if (shift) {
      included.delete(value);
      if (!left.delete(value)) {
        left.add(value);
      }
    } else if (left.delete(value)) {
      included.add(value);
    } else if (!included.delete(value)) {
      included.add(value);
    }
  }

  function update() {
    for (const item of items) {
      item.element.hidden = !matches(item);
    }
    for (const { button, facet, value, count, title } of buttons) {
      const n = paths(items.filter(item =>
        matches(item, facet) && (item.facets[facet] || []).includes(value)));
      count.textContent = n;
      button.classList.toggle("facet-empty", n == 0);
      const left = excluded.get(facet)?.has(value);
      button.classList.toggle("facet-excluded", !!left);
      if (toggles.has(facet)) {
        button.setAttribute("aria-pressed", left ? "false" : "true");
        button.title = `${title}${title ? " " : ""}(${left ? "left out; click to show them" : "shown; click to leave them out"})`;
        continue;
      }
      button.setAttribute("aria-pressed", selected.get(facet)?.has(value) ? "true" : "false");
      button.title = left
        ? "Left out (click to show only these; shift-click to stop leaving them out)"
        : "Click to show only these (and others selected); shift-click to leave them out";
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
    const filtering = [...selected.values(), ...excluded.values()].some(values => values.size);
    status.textContent = filtering ? `Showing ${shown} of ${plural(allPaths)}.` : "";
    clear.classList.toggle("facet-inactive", !filtering);
    saveState();
    // (For pages which show the items otherwise, ex: a diagram's nodes; see
    // graph.js.)
    container.dispatchEvent(new CustomEvent("facetschange"));
  }

  const compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
  const compareKeys = (a, b) => a.reduce((result, part, i) => result || compare(part, b[i]), 0);
  // The order of files by path (as `explore::file_order_key` in
  // tools/src/hyperblame/explore.rs), but with headers before the files with
  // their names (ex: "Foo.h" before "Foo.cpp", rather than after): by their
  // paths without their extensions, then whether they're headers.
  const HEADER_EXTENSIONS = new Set(["h", "hh", "hpp", "hxx"]);
  function fileOrderKey(path) {
    const nameStart = path.lastIndexOf("/") + 1;
    const dot = path.lastIndexOf(".");
    const stemEnd = dot > nameStart ? dot : path.length;
    const header = HEADER_EXTENSIONS.has(path.slice(stemEnd + 1));
    return [path.slice(0, stemEnd), header ? 0 : 1, path];
  }

  // Group the items by the chosen way of grouping, sorted by the groups' sort
  // keys and then by their names, and the items in each by path, or if
  // sorting by size, by their sizes, biggest first, and the groups by their
  // biggest items' sizes (rather than their totals, as many small changes
  // aren't a big one), except for path kinds, whose order says which matter
  // more (ex: tests after the code).
  function regroup() {
    const key = groupBy.value;
    const bySize = sortBy?.value == "size";
    const groups = new Map();
    for (const item of items) {
      const [sortKey, name] = item.groups[key] || ["", ""];
      if (!groups.has(name)) {
        groups.set(name, { sortKey, name, items: [], size: 0 });
      }
      const group = groups.get(name);
      group.items.push(item);
      group.size = Math.max(group.size, item.size);
    }
    const bySizeFirst = (a, b) => (bySize ? b.size - a.size : 0);
    const sections = [...groups.values()]
      .sort((a, b) => (key == "kind" ? 0 : bySizeFirst(a, b)) ||
        compare(a.sortKey, b.sortKey) || compare(a.name, b.name))
      .map(group => {
        const section = document.createElement("section");
        section.className = "facet-group";
        const header = document.createElement("h3");
        header.className = "facet-group-header";
        const count = document.createElement("span");
        count.className = "facet-group-count";
        header.append(`${group.name} `, count);
        group.items.sort((a, b) =>
          bySizeFirst(a, b) || compareKeys(fileOrderKey(a.path), fileOrderKey(b.path)));
        section.append(header, ...group.items.map(item => item.element));
        return section;
      });
    container.replaceChildren(...sections);
  }

  function saveState() {
    const params = new URLSearchParams(location.search);
    for (const key of [...params.keys()]) {
      if (key.startsWith("facet-") || key.startsWith("-facet-") || key == "group-by" || key == "sort-by") {
        params.delete(key);
      }
    }
    for (const [prefix, map] of [["facet-", selected], ["-facet-", excluded]]) {
      for (const [facet, values] of map) {
        // (Not a facet's values left out at first, if they're its only
        // choices.)
        if (isDefault(facet)) {
          continue;
        }
        for (const value of values) {
          params.append(`${prefix}${facet}`, value);
        }
      }
    }
    // (And a facet which leaves out values at first, but now none: an empty
    // left out value.)
    for (const facet of defaultExcluded.keys()) {
      if (!isDefault(facet) && !selected.get(facet)?.size && !excluded.get(facet)?.size) {
        params.append(`-facet-${facet}`, "");
      }
    }
    if (groupBy && groupBy.value != defaultGroupBy) {
      params.set("group-by", groupBy.value);
    }
    if (sortBy && sortBy.value != defaultSortBy) {
      params.set("sort-by", sortBy.value);
    }
    const query = params.toString();
    history.replaceState(history.state, "",
      location.pathname + (query ? `?${query}` : "") + location.hash);
  }

  // Whether a facet's choices are those it makes at first.
  function isDefault(facet) {
    const left = excluded.get(facet) || new Set();
    const initial = defaultExcluded.get(facet);
    return !!initial && !selected.get(facet)?.size && left.size == initial.size &&
      [...initial].every(value => left.has(value));
  }

  function loadState() {
    const params = new URLSearchParams(location.search);
    const mentioned = new Set();
    for (const [key, value] of params) {
      const left = key.startsWith("-facet-");
      const facet = (left || key.startsWith("facet-")) && key.slice(left ? "-facet-".length : "facet-".length);
      if (facet) {
        mentioned.add(facet);
      }
      if (facet && buttons.some(b => b.facet == facet && b.value == value)) {
        valuesOf(left ? excluded : selected, facet).add(value);
      }
    }
    // (The values the facets the URL doesn't mention leave out at first.)
    for (const [facet, values] of defaultExcluded) {
      if (!mentioned.has(facet)) {
        for (const value of values) {
          if (buttons.some(b => b.facet == facet && b.value == value)) {
            valuesOf(excluded, facet).add(value);
          }
        }
      }
    }
    for (const [select, param] of [[groupBy, "group-by"], [sortBy, "sort-by"]]) {
      const value = params.get(param);
      if (select && value && [...select.options].some(option => option.value == value)) {
        select.value = value;
      }
    }
  }

  // A value's nested values (`group` being its `.facet-value-group`).
  function showNested(group, shown) {
    const nested = group.querySelector(":scope > .facet-nested");
    if (!nested) {
      return;
    }
    nested.hidden = !shown;
    group.classList.toggle("facet-open", shown);
    const expand = group.querySelector(":scope > .facet-value-row > .facet-expand");
    expand.setAttribute("aria-expanded", shown ? "true" : "false");
    expand.querySelector(".facet-expand-icon").classList.toggle("expanded", shown);
  }

  // The elided values of the level of a "N more" button.
  function showMore(more, shown) {
    for (const group of more.parentElement.querySelectorAll(":scope > .facet-value-group[data-elided]")) {
      group.hidden = !shown;
    }
    more.setAttribute("aria-expanded", shown ? "true" : "false");
    more.textContent = shown ? "fewer" : `${more.dataset.more} more`;
  }

  // Show a value, if it's elided or in collapsed values.
  function reveal(button) {
    for (let group = button.closest(".facet-value-group"); group;
         group = group.parentElement.closest(".facet-value-group")) {
      group.hidden = false;
      const parent = group.parentElement.closest(".facet-value-group");
      if (parent) {
        showNested(parent, true);
      }
    }
  }

  function revealSelected() {
    for (const { button, facet, value } of buttons) {
      if (selected.get(facet)?.has(value) || excluded.get(facet)?.has(value)) {
        reveal(button);
      }
    }
  }

  bar.addEventListener("click", event => {
    const expand = event.target.closest(".facet-expand");
    const more = event.target.closest(".facet-more");
    const all = event.target.closest(".facet-expand-all, .facet-collapse-all");
    if (expand) {
      const group = expand.closest(".facet-value-group");
      const shown = expand.getAttribute("aria-expanded") != "true";
      showNested(group, shown);
      if (event.shiftKey) {
        for (const nested of group.querySelectorAll(".facet-value-group")) {
          showNested(nested, shown);
        }
        for (const nestedMore of group.querySelectorAll(".facet-more")) {
          showMore(nestedMore, shown);
        }
      }
      if (!shown) {
        revealSelected();
      }
      return;
    }
    if (more) {
      showMore(more, more.getAttribute("aria-expanded") != "true");
      revealSelected();
      return;
    }
    if (all) {
      const shown = all.classList.contains("facet-expand-all");
      const facet = all.closest(".facet");
      for (const group of facet.querySelectorAll(".facet-value-group")) {
        showNested(group, shown);
      }
      for (const facetMore of facet.querySelectorAll(".facet-more")) {
        showMore(facetMore, shown);
      }
      revealSelected();
      return;
    }
    const button = event.target.closest(".facet-value");
    if (button) {
      toggleValue(button.closest(".facet").dataset.facet, button.dataset.value, event.shiftKey);
      update();
    } else if (event.target.closest(".facet-clear")) {
      selected.clear();
      excluded.clear();
      update();
    }
  });
  for (const select of [groupBy, sortBy]) {
    select?.addEventListener("change", () => {
      regroup();
      update();
    });
  }

  // The groups' headers' context menu (see context-menu.js) has the choices
  // of grouping and sorting, as submenus, which check the current ones.
  let headerMenu = null;
  container.addEventListener("contextmenu", event => {
    if (!groupBy || !event.target.closest(".facet-group-header") ||
        typeof ContextMenu == "undefined") {
      return;
    }
    event.preventDefault();
    const choices = [[groupBy, "Group by"], [sortBy, "Sort by"]]
      .filter(([select]) => select);
    headerMenu ||= choices.map(([select, label]) => new MenuItemWithSubMenu({
      html: label,
      section: "facet-choices",
      menu: ContextMenu,
      searches: [],
      items: [...select.options].map(option => new MenuItem({
        html: ContextMenu.fmt("_", option.textContent),
        action: () => {
          ContextMenu.hide();
          select.value = option.value;
          select.dispatchEvent(new Event("change"));
        },
        icon: "ok",
        section: "facet-choice",
        option,
      })),
    }));
    for (const [i, [select]] of choices.entries()) {
      for (const item of headerMenu[i].items) {
        item.classNames = item.option.value == select.value ? [] : ["facet-choice-off"];
      }
    }
    ContextMenu.showItems(headerMenu, event);
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
    localStorage.setItem(storageKey("facets-collapsed"), content.hidden ? "1" : "0");
  });
  setExpanded(localStorage.getItem(storageKey("facets-collapsed")) != "1");

  // (For other scripts: whether a facet's value is left out, and leaving it
  // out or showing it.)
  window.ResultFacets = {
    container,
    isExcluded: (facet, value) => !!excluded.get(facet)?.has(value),
    setExcluded(facet, value, left) {
      if (left) {
        valuesOf(excluded, facet).add(value);
      } else {
        excluded.get(facet)?.delete(value);
      }
      update();
    },
  };

  loadState();
  revealSelected();
  if (groupBy && (groupBy.value != defaultGroupBy || (sortBy && sortBy.value != defaultSortBy))) {
    regroup();
  }
  update();
})();
