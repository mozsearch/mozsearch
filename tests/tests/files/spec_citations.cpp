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

// A section of an ecmarkup spec, cited by its number and title, as
// SpiderMonkey's comments cite ECMA-262's.
// ES2026 draft rev 0123456789abcdef0123456789abcdef01234567
// 1.1.1 ToString ( argument )
int StringOfWidget(Widget& aWidget) {
  // Step 1.
  return aWidget.state;
}

// The id Wattsi generates for a cross-reference, of its section's heading's
// and its target's, as Gecko cites the HTML Standard's.
// https://example.spec.whatwg.org/#gadgets:concept-widget-2
int CountWidgets(Widget* aWidgets, int aCount) {
  return aCount;
}

// A later level of the example spec (example-2, in its series), which doesn't
// define the term, so the citation is of the example spec's.
// https://example.spec.whatwg.org/2/#concept-widget
bool IsPoked(const Widget& aWidget) {
  return aWidget.state == 1;
}

// A citation by an outdated URL of the example spec (one of its `old_urls`
// in tests/tests/specs.toml), which says so.
// http://old.example.org/widgets/Overview.html#concept-widget
bool IsWidget(const Widget& aWidget) {
  return true;
}

}  // namespace spec_citations
