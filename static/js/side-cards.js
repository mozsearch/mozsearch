/**
 * Cards beside the blocks of a rendered document: a spec's algorithm view's
 * citations of its steps (see `format::write_algorithm_view`), and a spec's
 * page's pull requests' changes (see spec-pr-view.js).  `placeSideCards`
 * positions each column's cards (absolutely, in their column, in order), each
 * beside its block, or for one without, below the card before it, pushing the
 * blocks down with spaces where the cards before them need the room, and
 * stacking the cards beside the same block.  Its options:
 * - `doc`: the document, whose spaces (of the class `spaceClass`) it removes
 *   first;
 * - `columns`: each column's element (`aside`) and cards (`cards`, each's
 *   `element` and `block`), in order;
 * - `blocks`: the cards' blocks, in the document's order;
 * - `spaceBefore(block, previous)`: makes a space before a block (after the
 *   block before it with cards, `previous`), of the class `spaceClass`;
 * - `gap(card)`: the room above a card below another (ex: none for a card
 *   continuing the one before it);
 * - `beforePut(card, at)`: what a card shows (which changes its height) when
 *   it's put at `at` (ex: whether its header shows), if anything.
 * Cards of blocks later in the document than the next ones' (ex: of an
 * algorithm's steps cited out of order) go below the cards before them.
 */
window.placeSideCards = function ({ doc, columns, blocks, spaceClass, spaceBefore, gap, beforePut }) {
  for (const space of doc.querySelectorAll(`.${spaceClass}`)) {
    space.remove();
  }
  const order = new Map(blocks.map((block, i) => [block, i]));
  const tops = columns.map(({ aside }) => aside.getBoundingClientRect().top);
  const bottoms = columns.map(() => null);
  const next = columns.map(() => 0);
  const floor = (c, card) => (bottoms[c] === null ? 0 : bottoms[c] + gap(card));
  const put = (c, card, at) => {
    beforePut?.(card, at);
    card.element.style.top = `${at}px`;
    bottoms[c] = at + card.element.offsetHeight;
  };
  let previous = null;
  for (const block of blocks) {
    // (Each column's cards before its block's, but those of later blocks.)
    const goals = [];
    columns.forEach(({ cards }, c) => {
      while (next[c] < cards.length && cards[next[c]].block !== block &&
             !(order.get(cards[next[c]].block) > order.get(block))) {
        const card = cards[next[c]++];
        put(c, card, floor(c, card));
      }
      if (cards[next[c]]?.block === block) {
        goals.push(tops[c] + floor(c, cards[next[c]]));
      }
    });
    if (!goals.length) {
      continue;
    }
    const goal = Math.max(...goals);
    let top = block.getBoundingClientRect().top;
    if (goal > top + 1) {
      const space = spaceBefore(block, previous);
      space.style.height = `${Math.round(goal - top)}px`;
      // (Less its margins' collapsing.)
      const moved = block.getBoundingClientRect().top;
      space.style.height = `${Math.max(0, Math.round(2 * goal - top - moved))}px`;
      top = block.getBoundingClientRect().top;
    }
    columns.forEach(({ cards }, c) => {
      while (cards[next[c]]?.block === block) {
        const card = cards[next[c]++];
        put(c, card, Math.max(top - tops[c], floor(c, card)));
      }
    });
    previous = block;
  }
  columns.forEach(({ aside, cards }, c) => {
    while (next[c] < cards.length) {
      const card = cards[next[c]++];
      put(c, card, floor(c, card));
    }
    aside.style.minHeight = `${bottoms[c] ?? 0}px`;
  });
};
