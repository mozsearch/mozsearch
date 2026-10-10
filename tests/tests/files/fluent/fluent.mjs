/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

const l10n = new Localization(["fluent/test.ftl"], true);

export function insertLocalization(win) {
  win.MozXULElement.insertFTLIfNeeded("fluent/test.ftl");
}

export async function closeWarning(elem, count) {
  document.l10n.setAttributes(elem, "tabs-close-warning", { count });
  const [title, label] = await l10n.formatValues([
    { id: "tabs-close-warning", args: { "count": count } },
    "tabs-close-button",
  ]);
  elem.setAttribute("data-l10n-id", "tabs-close-button");
  elem.title = title;
  return label + l10n.formatValueSync("about-link");
}

export const BUTTON = {
  l10nId: "tabs-close-button",
  titleL10nId: "tabs-close-warning",
  titleL10nArgs: { count: 1 },
};

export function template(html, items) {
  return html`<button data-l10n-id="tabs-close-button">
    ${items.map(item => html`<li data-l10n-id="about-link">${item}</li>`)}
  </button>`;
}

export const MARKUP = `<span data-l10n-id="about-link"></span>`;
export const ESCAPED = "<span data-l10n-id=\"tabs-close-button\"></span>";
