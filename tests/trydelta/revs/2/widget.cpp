#include "widget.h"

namespace trydelta {

void Widget::Frob(int aAmount) { mSize += aAmount; }

void Widget::Shrink() {
  if (mSize > 0) {
    mSize--;
  }
}

}  // namespace trydelta
