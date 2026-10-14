#include "widget.h"

namespace trydelta {

// https://example.spec.whatwg.org/multipage/widgets.html#unpoke
void Unpoke(Widget& aWidget) {
  // (Back to nothing, however much it was frobbed.)
  // Step 1. Set widget's state to "idle".
  aWidget.Frob(-aWidget.Size(), false);
}

}  // namespace trydelta
