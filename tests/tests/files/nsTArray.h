// This file is a combination of a stub to let xpctest_params.h compile and a
// recognition that we already had `templates_nsTArray.cpp` that defined an
// nsTArray and so to avoid symbol-space collisions, that needed to be
// abstracted up into here.

#ifndef nsTArray_h__
#define nsTArray_h__

template <class T>
class Span {
 private:
  T* mRawPtr;
};

template <class E>
class nsTArray {
  template <class Item>
  E* AppendElements(const Item* aArray, unsigned aArrayLen) {
    return nullptr;
  }

  template <class Item>
  E* AppendElements(Span<const Item> aSpan) {
    return AppendElements<Item>(aSpan.Elements(), aSpan.Length());
  }
};

#endif  // nsTArray_h__
