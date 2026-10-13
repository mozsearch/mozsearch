#pragma once

#include "widget.h"

namespace trydelta {

// Frob a widget twice, gadget-style.
inline void FrobTwice(Widget& aWidget) {
  aWidget.Frob(1);
  aWidget.Frob(1);
}

}  // namespace trydelta
