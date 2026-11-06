/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A bug comment's view (see the tools' `bug_comment::render_bug_comment`),
// which loads this after its HTML.
// (A card and what in its block's text it's of are marked while either is
// hovered, to tell which is which: a code link's card, its link; a crash
// report's, its report's links; listed frames', their code block.)
let paired = [];
function cardOf(section, card) {
  const links = [...section.querySelectorAll(".bug-comment-text a[href]")];
  if (card.dataset.link !== undefined) {
    return links.filter(link => link.getAttribute("href") == card.dataset.link);
  }
  if (card.dataset.crash) {
    return links.filter(link => link.getAttribute("href").includes(card.dataset.crash));
  }
  if (card.dataset.frames !== undefined) {
    return [...section.querySelectorAll(".bug-comment-text pre")];
  }
  return [];
}
document.addEventListener("mouseover", event => {
  const section = event.target.closest(".bug-comment-block");
  const card = event.target.closest(".bug-comment-cards > .bug-comment-card");
  const link = event.target.closest(".bug-comment-text a[href], .bug-comment-text pre");
  let marks = [];
  if (section && card) {
    marks = [card, ...cardOf(section, card)];
  } else if (section && link) {
    const cards = [...section.querySelectorAll(".bug-comment-cards > .bug-comment-card")]
      .filter(card => cardOf(section, card).includes(link));
    if (cards.length) {
      marks = [link, ...cards];
    }
  }
  if (marks.length == paired.length && marks.every((mark, i) => mark == paired[i])) {
    return;
  }
  for (const mark of paired) {
    mark.classList.remove("bug-comment-paired");
  }
  for (const mark of marks) {
    mark.classList.add("bug-comment-paired");
  }
  paired = marks;
});
// (A block's text sticks below the page's header while its cards scroll by,
// unless it's too tall to show at once.)
function fitProse() {
  const header = document.getElementById("fixed-header")?.offsetHeight || 0;
  for (const prose of document.querySelectorAll(".bug-comment-prose")) {
    prose.classList.toggle("bug-comment-prose-tall", prose.offsetHeight > window.innerHeight - header - 16);
  }
}
fitProse();
window.addEventListener("resize", fitProse);
// (The comment's time, as Bugzilla shows it: how long ago it was, its time in
// its title.)
for (const time of document.querySelectorAll(".bmo-change-time[datetime]")) {
  const seconds = (Date.now() - Date.parse(time.getAttribute("datetime"))) / 1000;
  const units = [["year", 365 * 86400], ["month", 30 * 86400], ["day", 86400], ["hour", 3600], ["minute", 60]];
  const [unit, size] = units.find(([, size]) => seconds >= size) || [];
  const count = unit && Math.floor(seconds / size);
  time.textContent = unit ? `${count} ${unit}${count == 1 ? "" : "s"} ago` : "just now";
}
