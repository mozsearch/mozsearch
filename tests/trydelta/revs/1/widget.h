#pragma once

namespace trydelta {

// A widget, which can be frobbed.
class Widget {
 public:
  explicit Widget(int aSize) : mSize(aSize) {}

  // Frob the widget by an amount.
  void Frob(int aAmount);

  int Size() const { return mSize; }

 private:
  int mSize;
};

}  // namespace trydelta
