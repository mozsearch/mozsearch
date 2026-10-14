#include "widget.h"

namespace trydelta {

// https://example.spec.whatwg.org/#concept-widget-poke
void Poke(Widget& aWidget) {
  // Step 1. Set widget's state to "poked".
  aWidget.Frob(1, true);
  // Step 2. Return.
}

}  // namespace trydelta
