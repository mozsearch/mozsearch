#include "widget.h"

namespace trydelta {

void Idle(Widget& aWidget);

// https://example.spec.whatwg.org/multipage/widgets.html#unpoke
void Unpoke(Widget& aWidget) { Idle(aWidget); }

}  // namespace trydelta
