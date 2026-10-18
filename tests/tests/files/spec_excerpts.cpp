// The excerpts of code citing an algorithm's steps on its view (see
// format::citation_extent and format::mark_continuations): a step's code goes
// on past comments and blank lines, folding when it's long, and the next
// step's card, in the same function, continues it, with the few lines between
// them, or how many there are.

namespace spec_excerpts {

int Make();
void Frob(int aValue);

// https://example.spec.whatwg.org/#assemble
void AssembleAll(int* aWidgets, int aCount) {
  int node = 0;
  if (aCount) {
    // Step 1. Let node be a new node.
    node = Make();
  }
  Frob(node);

  // Step 2. For each widget in it:
  // (Its comment and the blank line after it don't end its code, nor does
  // its length: it folds.)

  for (int i = 0; i < aCount; i++) {
    Frob(aWidgets[i] + 1);
    Frob(aWidgets[i] + 2);
    Frob(aWidgets[i] + 3);
    Frob(aWidgets[i] + 4);
    Frob(aWidgets[i] + 5);
    Frob(aWidgets[i] + 6);
    Frob(aWidgets[i] + 7);
    Frob(aWidgets[i] + 8);
    Frob(aWidgets[i] + 9);
    Frob(aWidgets[i] + 10);
    Frob(aWidgets[i] + 11);
    Frob(aWidgets[i] + 12);
  }
  if (aCount > 1) {
    // Step 2.1. Poke it.
    Frob(aWidgets[0]);
  }
  Frob(1);
  Frob(2);
  Frob(3);
  Frob(4);
  // Step 2.2. See widgets and trees.
  Frob(5);
}

// https://example.github.io/bs/#poke
void PokeTwice(int aWidget) {
  if (aWidget) {
    // Step 1. Set widget's state to "poked".
    Frob(aWidget);
  }
  Frob(1);
  Frob(2);
  Frob(3);
  Frob(4);
  // Step 2. Return.
}

}  // namespace spec_excerpts
