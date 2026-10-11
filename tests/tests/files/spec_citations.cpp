// Citations of web specs (see tools/src/spec_analyze/citations.rs): URLs to
// the example spec's algorithms, and the comments of their steps in the
// functions implementing them.

namespace spec_citations {

struct Widget {
  int state = 0;
};

// https://example.spec.whatwg.org/#concept-widget-poke
void Poke(Widget& aWidget) {
  // Step 1. Set widget's state to "poked".
  aWidget.state = 1;
  // Step 2.
}

// https://example.spec.whatwg.org/multipage/widgets.html#unpoke
bool
Unpoke(Widget& aWidget)
{
  // Step 1.
  aWidget.state = 0;
  return true;
}

// Step 3.  (Not a step of either: it's in neither's scope.)
void Assemble(Widget& aWidget) {
  // https://example.spec.whatwg.org/#assemble
  // Step 2.1.
  Poke(aWidget);
}

}  // namespace spec_citations
