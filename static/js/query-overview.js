/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

/**
 * The overview of /query/'s results of methods in an override hierarchy, in
 * the navigation panel (the "overview" lens; see navigation_panel.liquid, and
 * `ResultsOverview` in the tools' cmd_pipeline/interface.rs): the methods the
 * queried ones override above them, and the queried ones below those, each
 * with how many definitions, declarations and uses it has, and a link to its
 * own results; and below each method, its overrides in a row of their own,
 * the queried ones' (which lists them, open if they're a few) and its other
 * ones (the queried ones' cousins, of which there can be hundreds, with their
 * own overrides, listed when it's opened).  A method overridden by too many
 * for the results to follow (ex: `nsISupports::AddRef`) says so, as its uses
 * aren't in the results, with a link to the results following them anyway.  Its buttons show or leave out the results of each
 * relation, as the "Relation" facet's values do (see facets.js's
 * `ResultFacets`), whose changes they follow.
 */
(function () {
  const container = document.querySelector(".panel-overview .overview-tree");
  const nodes = window.RESULTS_OVERVIEW?.nodes;
  if (!container || !nodes?.length) {
    return;
  }
  const tree = document.getElementById("data")?.getAttribute("data-tree");
  // How many of the queried methods' overrides are listed open.
  const OVERRIDES_OPEN = 5;

  const bySym = new Map(nodes.map(node => [node.sym, node]));
  const children = new Map();
  for (const node of nodes) {
    for (const parent of node.overrides) {
      if (!children.has(parent)) {
        children.set(parent, []);
      }
      children.get(parent).push(node);
    }
  }
  // A method and its overrides, and theirs.
  function subtree(node, seen = new Set()) {
    if (seen.has(node.sym)) {
      return [];
    }
    seen.add(node.sym);
    return [node, ...(children.get(node.sym) || []).flatMap(child => subtree(child, seen))];
  }
  const byPretty = (a, b) => (a.pretty < b.pretty ? -1 : a.pretty > b.pretty ? 1 : 0);

  // The relations the results have (the "Relation" facet's values).
  const relations = new Set([...document.querySelectorAll('.facet[data-facet="relation"] .facet-value')]
    .map(value => value.dataset.value));

  // A method's name (its class's and its own; its whole name is its
  // tooltip), as a link to its own results.
  function nameLink(node) {
    const link = document.createElement("a");
    link.className = "overview-name";
    link.href = `/${tree}/query/default?q=${encodeURIComponent(`symbol:${node.sym}`)}`;
    link.textContent = node.pretty.split("::").slice(-2).join("::");
    link.title = `${node.pretty} (its own results)`;
    return link;
  }

  function counts(node) {
    const parts = [];
    for (const [count, one, many] of [[node.definitions, "def", "defs"],
                                      [node.declarations, "decl", "decls"],
                                      [node.uses, "use", "uses"]]) {
      if (count) {
        parts.push(`${count} ${count == 1 ? one : many}`);
      }
    }
    // (Of a method the queried ones override, how many of its uses are calls
    // from its overrides, rather than possible virtual calls.)
    if (node.super_calls) {
      parts[parts.length - 1] += ` (${node.super_calls} from overrides)`;
    }
    const span = document.createElement("span");
    span.className = "overview-counts";
    span.textContent = parts.join(", ");
    return span;
  }

  // A button showing or leaving out a relation's results, if the results
  // have any (see `sync`).
  const toggles = [];
  function toggle(relation, label, title) {
    if (!relations.has(relation)) {
      return [];
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "overview-toggle";
    button.dataset.relation = relation;
    button.textContent = label;
    button.title = title;
    button.setAttribute("aria-pressed", "false");
    button.addEventListener("click", event => {
      // (Not opening or closing the row it's in.)
      event.preventDefault();
      window.ResultFacets?.setExcluded("relation", relation,
                                       button.getAttribute("aria-pressed") == "true");
    });
    toggles.push(button);
    return [button];
  }

  function sync() {
    for (const button of toggles) {
      const shown = !window.ResultFacets?.isExcluded("relation", button.dataset.relation);
      button.setAttribute("aria-pressed", shown ? "true" : "false");
    }
  }

  function row(node, kind) {
    const item = document.createElement("li");
    item.className = `overview-node overview-${kind}`;
    const line = document.createElement("div");
    line.className = "overview-row";
    line.append(nameLink(node), " ", counts(node));
    if (node.too_many_overrides) {
      const note = document.createElement("span");
      note.className = "overview-note";
      note.textContent = `overridden by ${node.too_many_overrides}, too many to follow: its uses aren't in the results; `;
      // (The query again, without the limit on overrides; see the query
      // language's `override-limit:`.)
      const query = document.querySelector("#query")?.value.replace(/\s*\boverride-limit:\S*/g, "") || "";
      const anyway = document.createElement("a");
      anyway.href = `/${tree}/query/default?q=${encodeURIComponent(`${query} override-limit:0`.trim())}`;
      anyway.textContent = "follow them anyway";
      note.append(anyway);
      line.append(" ", note);
    }
    item.append(line);
    return item;
  }

  // A row of overrides: their count, and the button showing or leaving out
  // their results, which lists them when opened.
  function overridesRow(list, members, relation, label, title) {
    const all = [...new Set(members.flatMap(member => subtree(member)))].sort(byPretty);
    const item = document.createElement("li");
    item.className = `overview-node overview-${relation}`;
    const details = document.createElement("details");
    details.open = relation == "overrides" && all.length <= OVERRIDES_OPEN;
    const summary = document.createElement("summary");
    summary.append(`${all.length} ${label(all.length)} `, ...toggle(relation, "results", title));
    const inner = document.createElement("ul");
    for (const node of all) {
      inner.append(row(node, relation));
    }
    details.append(summary, inner);
    item.append(details);
    list.append(item);
  }

  const drawn = new Set();
  function draw(node, list) {
    if (drawn.has(node.sym)) {
      return;
    }
    drawn.add(node.sym);
    const item = row(node, node.relation == "self" ? "queried" : node.relation);
    if (node.relation == "overridden") {
      item.querySelector(".overview-row").append(
        " ",
        ...toggle("virtual-uses", "uses",
                  "Show or leave out the uses of the methods the queried ones override (possible virtual calls of them)"),
        ...toggle("overridden", "definition",
                  "Show or leave out the definitions and declarations of the methods the queried ones override"));
    }
    list.append(item);
    const kids = children.get(node.sym) || [];
    const nested = document.createElement("ul");
    // (The queried methods, and the methods between them and this one, in
    // rows of their own; this one's other overrides in one.)
    for (const kid of kids.filter(kid => kid.relation == "self" || kid.relation == "overridden")) {
      draw(kid, nested);
    }
    const overrides = kids.filter(kid => kid.relation == "overrides");
    if (overrides.length) {
      overridesRow(nested, overrides, "overrides",
                   n => (n == 1 ? "override of it" : "overrides of it"),
                   "Show or leave out the results of the queried methods' overrides");
    }
    const cousins = kids.filter(kid => kid.relation == "cousins");
    if (cousins.length) {
      overridesRow(nested, cousins, "cousins",
                   n => (n == 1 ? "other override (a cousin)" : "other overrides (cousins)"),
                   "Show or leave out the results of the other overrides of the methods the queried ones override");
    }
    if (nested.children.length) {
      item.append(nested);
    }
  }

  // (From the methods overriding none of the others: the queried ones, or
  // the methods they override.)
  const list = document.createElement("ul");
  for (const node of nodes) {
    const parents = node.overrides.filter(parent => bySym.has(parent));
    if (!parents.length && (node.relation == "self" || node.relation == "overridden")) {
      draw(node, list);
    }
  }
  container.append(list);

  // (Once the facets are there, whose script comes after.)
  document.addEventListener("DOMContentLoaded", () => {
    sync();
    window.ResultFacets?.container.addEventListener("facetschange", sync);
  });
})();
