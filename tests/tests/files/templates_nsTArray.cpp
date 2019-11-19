template <class T>
class Span
{
private:
  T* mRawPtr;
};

template<class E>
class nsTArray {
  template<class Item>
  E* AppendElements(const Item* aArray, unsigned aArrayLen)
  {
    return nullptr;
  }

  template<class Item>
  E* AppendElements(Span<const Item> aSpan)
  {
    return AppendElements<Item>(aSpan.Elements(), aSpan.Length());
  }
};

struct ServoAttrSnapshot {};

class ServoElementSnapshot
{
  nsTArray<ServoAttrSnapshot> mAttrs;
};
