#pragma once

namespace trydelta {

// A widget, which can be frobbed.
class Widget {
 public:
  explicit Widget(int aSize) : mSize(aSize) {}

  // Frob the widget by an amount, loudly if asked to.
  void Frob(int aAmount, bool aLoudly = false);

  // Shrink the widget, but not below nothing.
  void Shrink();

  // Wobble the widget.
  void Wobble();

  int Size() const { return mSize; }

 private:
  int mSize;
  int mWobbles = 0;
};

}  // namespace trydelta
