#include "widget.h"

namespace trydelta {

void Widget::Frob(int aAmount, bool aLoudly) {
  mSize += aLoudly ? aAmount * 2 : aAmount;
}

void Widget::Shrink() {
  if (mSize > 0) {
    mSize--;
  }
}

void Widget::Wobble() { mWobbles++; }

}  // namespace trydelta
