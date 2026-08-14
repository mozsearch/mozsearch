// Bug 2063622 was a crash in the Clang plugin when reaching dependent template names.

template <typename T, typename U>
void test() {
  typename T::template Dependent<U>();
}

// ↑ this was enough to get it to crash (no usage required)

class Class {
public:
  template<typename T>
  class Dependent {};
};

void func() {
  test<Class, int>();
}
