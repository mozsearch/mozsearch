template<typename T>
class nsTArray {
public:
  nsTArray(const T& aT) : myT(aT) {}

  template<typename Foo = T>
  bool Contains(T aT, Foo foo) {
    return myT == aT || myT == foo;
  }
private:
  T myT;
};
