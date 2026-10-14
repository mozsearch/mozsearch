#include "widget.h"

namespace trydelta {

// https://example.spec.whatwg.org/multipage/widgets.html#unpoke
void Unpoke(Widget& aWidget) {
  // Step 1. Set widget's state to "idle".
  aWidget.Frob(-1);
}

}  // namespace trydelta
