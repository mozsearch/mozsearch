namespace nesting_initializer {

int shortList[] = {
  1,
  2,
  3,
  4,
  5,
  6,
};

int longList[] = {
  1,
  2,
  3,
  4,
  5,
  6,
  7,
  8,
  9,
  10,
};

template <typename Func>
bool callLambda(Func func) {
  return func();
}

bool foo() {
  bool result1 = callLambda([] () {
    // This is a short function.
    // 2
    // 3
    // 4
    // 5
    // 6
    return true;
  });
  bool result2 = callLambda([] () {
    // This is a long function.
    // 2
    // 3
    // 4
    // 5
    // 6
    // 7
    // 8
    // 9
    // 10
    // 11
    // 12
    // 13
    return true;
  });
  return result1 && result2;
}

/*
 * Some lines to make this file scrollable.
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 *
 */

}
