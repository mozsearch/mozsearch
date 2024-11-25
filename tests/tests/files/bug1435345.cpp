/* Searchfox should analyze uses of operator== */

struct Foo {
  Foo(int aX) : mX(aX) {}
  bool operator==(const Foo& aOther) { return mX == aOther.mX; }

  int mX;
};

int main(int argc, char** argv) {
  Foo a(0);
  Foo b(1);
  return a == b;
}
