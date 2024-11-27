struct SomeStruct {
  static void some_function();
};

template <typename T, typename U>
struct SomeTemplate {};

template <typename U>
struct SomeTemplate<int, U> {
  static void call_function() { U::some_function(); }
};

void test() { SomeTemplate<int, SomeStruct>::call_function(); }
