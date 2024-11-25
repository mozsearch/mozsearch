class Foo {
 public:
  void Method();
};

template <class T>
class Bar {
 public:
  void Function1();

  void Function2() { field->Method(); }

 private:
  T* field;
};

template <class T>
inline void Bar<T>::Function1() {
  return field->Method();
}

template class Bar<Foo>;

class Baz {
 public:
  void Method();
};

int main() {
  Foo* f;
  Bar<Baz>* b;
  b->Function1();
  b->Function2();
  return 0;
}
