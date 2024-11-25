class Foo {
 public:
  void Method();
};

class Bar {
 public:
  template <class T>
  void Function(T* t);
};

template <class T>
inline void Bar::Function(T* t) {
  t->Method();
}

int main() {
  Foo* f;
  Bar* b;
  b->Function(f);
  return 0;
}
