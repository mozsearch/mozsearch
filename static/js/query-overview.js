/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

/**
 * The overview of /query/'s results of methods in an override hierarchy, or
 * of classes in an inheritance hierarchy, in the navigation panel (the
 * "overview" lens; see navigation_panel.liquid, and `ResultsOverview` in the
 * tools' cmd_pipeline/interface.rs): the methods the queried ones override
 * (or the classes they inherit from) above them, and the queried ones below
 * those, each with how many definitions, declarations and uses it has, and a
 * link to its own results; and below each, its overrides (or subclasses) in
 * a row of their own, the queried ones' (which lists them, open if they're a
 * few) and its other ones (the queried ones' cousins, of which there can be
 * hundreds, with their own, listed when it's opened).  A method overridden
 * (or a class subclassed) by too many for the results to follow (ex:
 * `nsISupports::AddRef`) says so, as a method's uses then aren't in the
 * results, with a link to the results following them anyway.
 *
 * A method's buttons show or leave out its results of each relation, and a
 * row of overrides' buttons theirs, as the "Relation" facet's values do for
 * all of a relation's (see facets.js's `ResultFacets`, and its values'
 * members), whose changes they follow.  (A class's relatives' results aren't
 * the queried class's, so its rows only link to theirs.)
 */
(function () {
  const container = document.querySelector(".panel-overview .overview-tree");
  const nodes = window.RESULTS_OVERVIEW?.nodes;
  if (!container || !nodes?.length) {
    return;
  }
  const tree = document.getElementById("data")?.getAttribute("data-tree");
  // How many of the queried methods' overrides (or classes' subclasses) are
  // listed open.
  const DESCENDANTS_OPEN = 5;

  const bySym = new Map(nodes.map(node => [node.sym, node]));
  const children = new Map();
  for (const node of nodes) {
    for (const parent of node.parents) {
      if (!children.has(parent)) {
        children.set(parent, []);
      }
      children.get(parent).push(node);
    }
  }
  // A node and its descendants.
  function subtree(node, seen = new Set()) {
    if (seen.has(node.sym)) {
      return [];
    }
    seen.add(node.sym);
    return [node, ...(children.get(node.sym) || []).flatMap(child => subtree(child, seen))];
  }
  const byPretty = (a, b) => (a.pretty < b.pretty ? -1 : a.pretty > b.pretty ? 1 : 0);

  // The symbols whose results the page has, by their relation (see
  // kind_group.liquid's `data-facet-scope` and line_span.liquid's
  // `data-syms`), which the buttons show or leave out.
  const resultSyms = new Map();
  for (const line of document.querySelectorAll(".facet-items [data-syms]")) {
    const scope = line.closest("[data-facet-scope]");
    for (const relation of JSON.parse(scope?.dataset.facetScope || "{}").relation || []) {
      if (!resultSyms.has(relation)) {
        resultSyms.set(relation, new Set());
      }
      for (const sym of line.dataset.syms.split(" ")) {
        resultSyms.get(relation).add(sym);
      }
    }
  }

  // A method's or class's name (its class's and its own, or its own; its
  // whole name is its tooltip), as a link to its own results.
  function nameLink(node) {
    const link = document.createElement("a");
    link.className = "overview-name";
    link.href = `/${tree}/query/default?q=${encodeURIComponent(`symbol:${node.sym}`)}`;
    link.textContent = node.pretty.split("::").slice(node.class ? -1 : -2).join("::");
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

  // A button showing or leaving out some symbols' results of a relation, if
  // the page has any of theirs (see `sync`): all of them if some are left
  // out, or else none.
  const toggles = [];
  function toggle(relation, syms, label, title) {
    syms = syms.filter(sym => resultSyms.get(relation)?.has(sym));
    if (!syms.length) {
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
      const facets = window.ResultFacets;
      facets?.setMembersHidden("relation", relation, syms,
                               facets.valueState("relation", relation, syms) == "shown");
    });
    toggles.push({ button, relation, syms });
    return [button];
  }

  function sync() {
    for (const { button, relation, syms } of toggles) {
      const state = window.ResultFacets?.valueState("relation", relation, syms);
      button.setAttribute("aria-pressed", state == "shown" ? "true" : state == "mixed" ? "mixed" : "false");
    }
  }

  // The words for a method's and a class's relatives.
  const WORDS = {
    method: {
      overridden: "overridden",
      descendants: n => (n == 1 ? "override of it" : "overrides of it"),
      cousins: n => (n == 1 ? "other override (a cousin)" : "other overrides (cousins)"),
      limit: "override-limit",
    },
    class: {
      overridden: "subclassed",
      descendants: n => (n == 1 ? "subclass" : "subclasses"),
      cousins: n => (n == 1 ? "other subclass (a cousin)" : "other subclasses (cousins)"),
      limit: "subclass-limit",
    },
  };
  const wordsOf = node => WORDS[node.class ? "class" : "method"];

  // The query again, with a limit term (see the query language's
  // `override-limit:` and `subclass-limit:`) of 0.
  function unlimited(term) {
    const query = document.querySelector("#query")?.value
      .replace(new RegExp(`\\s*\\b${term}:\\S*`, "g"), "") || "";
    return `/${tree}/query/default?q=${encodeURIComponent(`${query} ${term}:0`.trim())}`;
  }

  // The results' relation of each of a method's relation's buttons, and their
  // labels and titles.
  function methodToggles(node, relation, several) {
    const sym = [node.sym];
    switch (relation) {
      case "self":
        return several
          ? toggle("self", sym, "results", `Show or leave out ${node.pretty}'s results`)
          : [];
      case "ancestor":
        return [
          ...toggle("virtual-uses", sym, "uses",
                    `Show or leave out the uses of ${node.pretty} (possible virtual calls of the queried methods)`),
          ...toggle("overridden", sym, node.super_calls ? "definition, super calls" : "definition",
                    `Show or leave out the definitions and declarations of ${node.pretty}` +
                    (node.super_calls ? ", and the calls of it from its overrides" : "")),
        ];
      case "descendant":
        return toggle("overrides", sym, "results", `Show or leave out ${node.pretty}'s results`);
      case "cousin":
        return toggle("cousins", sym, "results", `Show or leave out ${node.pretty}'s results`);
    }
    return [];
  }

  const severalQueried = nodes.filter(node => node.relation == "self" && !node.class).length > 1;
  function row(node, relation) {
    const item = document.createElement("li");
    item.className = `overview-node overview-${relation == "self" ? "queried" : relation}`;
    const line = document.createElement("div");
    line.className = "overview-row";
    line.append(nameLink(node), " ", counts(node));
    if (!node.class) {
      const buttons = methodToggles(node, relation, severalQueried);
      if (buttons.length) {
        line.append(" ", ...buttons);
      }
    }
    const words = wordsOf(node);
    if (node.too_many) {
      const note = document.createElement("span");
      note.className = "overview-note";
      // (A method the queried ones override's uses could be calls of any of
      // them; see `ResultRelation::VirtualUses`.)
      const dropped = !node.class && relation == "ancestor" ? ": its uses aren't in the results" : "";
      note.textContent = `${words.overridden} by ${node.too_many}, too many to follow${dropped}; `;
      const anyway = document.createElement("a");
      anyway.href = unlimited(words.limit);
      anyway.textContent = "follow them anyway";
      note.append(anyway);
      line.append(" ", note);
    } else if (node.over_budget) {
      const note = document.createElement("span");
      note.className = "overview-note";
      note.textContent = `${words.overridden} by ${node.over_budget}, not followed (the limit on all of the results' ones)`;
      line.append(" ", note);
    }
    item.append(line);
    return item;
  }

  // A row of descendants or cousins: their count, and the button showing or
  // leaving out their results (a method's), which lists them when opened.
  function relativesRow(list, members, relation, label) {
    const all = [...new Set(members.flatMap(member => subtree(member)))].sort(byPretty);
    const item = document.createElement("li");
    item.className = `overview-node overview-${relation}`;
    const details = document.createElement("details");
    details.open = relation == "descendant" && all.length <= DESCENDANTS_OPEN;
    const summary = document.createElement("summary");
    summary.append(`${all.length} ${label(all.length)} `);
    if (!all[0].class) {
      const results = relation == "descendant" ? "overrides" : "cousins";
      summary.append(...toggle(results, all.map(node => node.sym), "results",
                               `Show or leave out the results of ${all.length == 1 ? "it" : `all ${all.length}`}`));
    }
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
    // (A node with several parents, ex: a class inheriting from two, is
    // under the first, and says so under the others.)
    if (drawn.has(node.sym)) {
      const item = document.createElement("li");
      item.className = "overview-node overview-again";
      item.append(nameLink(node), " ");
      const note = document.createElement("span");
      note.className = "overview-note";
      note.textContent = "(above)";
      item.append(note);
      list.append(item);
      return;
    }
    drawn.add(node.sym);
    const item = row(node, node.relation);
    list.append(item);
    const kids = children.get(node.sym) || [];
    const nested = document.createElement("ul");
    const words = wordsOf(node);
    // (The queried ones, and those between them and this one, in rows of
    // their own; this one's other descendants in one.)
    for (const kid of kids.filter(kid => kid.relation == "self" || kid.relation == "ancestor")) {
      draw(kid, nested);
    }
    const descendants = kids.filter(kid => kid.relation == "descendant");
    if (descendants.length) {
      relativesRow(nested, descendants, "descendant", words.descendants);
    }
    const cousins = kids.filter(kid => kid.relation == "cousin");
    if (cousins.length) {
      relativesRow(nested, cousins, "cousin", words.cousins);
    }
    if (nested.children.length) {
      item.append(nested);
    }
  }

  // (From the nodes without parents among them: the queried ones, or what
  // they override or inherit from; the methods' first, then the classes',
  // each under a heading if there are both; but not those of only queried
  // symbols without relatives, ex: a queried class's constructor.)
  const roots = nodes.filter(node => !node.parents.some(parent => bySym.has(parent)) &&
    (node.relation == "self" || node.relation == "ancestor"));
  const lone = node => node.relation == "self" && !children.has(node.sym) &&
    !node.too_many && !node.over_budget;
  const kinds = [["Methods", roots.filter(node => !node.class)],
                 ["Classes", roots.filter(node => node.class)]]
    .filter(([, kindRoots]) => !kindRoots.every(lone));
  for (const [heading, kindRoots] of kinds) {
    if (kinds.length > 1) {
      const h5 = document.createElement("h5");
      h5.textContent = heading;
      container.append(h5);
    }
    const list = document.createElement("ul");
    for (const node of kindRoots) {
      draw(node, list);
    }
    container.append(list);
  }

  // (Once the facets are there, whose script comes after.)
  document.addEventListener("DOMContentLoaded", () => {
    sync();
    window.ResultFacets?.container.addEventListener("facetschange", sync);
  });
})();
