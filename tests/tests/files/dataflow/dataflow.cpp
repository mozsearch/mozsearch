// Fixtures for following values (see tools/src/dataflow/ and the checks in
// tests/tests/checks/inputs/dataflow/): out-parameters, fields, macros,
// templates' overloads, and an XPIDL method's implementation, which JS calls
// in ts/src/dataflow.js.

#include "dist/include/xpctest_params.h"

#define DF_ENSURE_SUCCESS(aRv, aRet) \
  do {                               \
    if ((aRv) != 0) {                \
      return aRet;                   \
    }                                \
  } while (false)

static int LookupBool(const char* aName, bool* aResult) {
  if (!aName) {
    return 1;
  }
  *aResult = aName[0] == 't';
  return 0;
}

int GetBoolWithDefault(const char* aName, bool aDefault, bool* aRetVal) {
  int rv = LookupBool(aName, aRetVal);
  if (rv != 0) {
    *aRetVal = aDefault;
  }
  return 0;
}

int CheckedLookup(const char* aName, bool* aRetVal) {
  int rv = LookupBool(aName, aRetVal);
  DF_ENSURE_SUCCESS(rv, rv);
  return 0;
}

bool UseDefault() {
  bool value = false;
  GetBoolWithDefault("browser.thing", true, &value);
  return value;
}

class Widget {
 public:
  explicit Widget(const char* aName) : mName(aName), mCount(0) {}
  const char* Name() const { return mName; }
  void Bump(int aBy) { mCount += aBy; }
  int Count() const { return mCount; }

 private:
  const char* mName;
  int mCount;
};

const char* MakeWidgetName() {
  Widget widget("gadget");
  return widget.Name();
}

int BumpedCount(int aBy) {
  Widget widget("counter");
  widget.Bump(aBy);
  return widget.Count();
}

// Overloads which a template's call reaches, per instantiation.
void Store(bool* aOut) { *aOut = true; }
void Store(int* aOut) { *aOut = 42; }

template <typename T>
void StoreThrough(T&& aOut) {
  Store(static_cast<T&&>(aOut));
}

bool StoreBool() {
  bool flag = false;
  StoreThrough(&flag);
  return flag;
}

int StoreInt() {
  int n = 0;
  StoreThrough(&n);
  return n;
}

// A method a macro defines.
#define DF_GETTER(aType, aName) \
  aType Get##aName() const { return m##aName; }

class Holder {
 public:
  explicit Holder(int aSize) : mSize(aSize) {}
  DF_GETTER(int, Size)

 private:
  int mSize;
};

int HolderSize() {
  Holder holder(7);
  return holder.GetSize();
}

// An XPIDL method's C++ implementation (abstract, as it implements only
// this method).
class DataflowParams : public nsIXPCTestParams {
 public:
  NS_IMETHOD TestBoolean(bool a, bool* b, bool* _retval) override;
};

uint32_t DataflowParams::TestBoolean(bool a, bool* b, bool* _retval) {
  *_retval = a && *b;
  *b = a;
  return 0;
}

// A lambda's init-capture: a variable of its own, which its body uses (ex:
// `[aScopeURL = nsCString(aScopeURL)]` in a runnable's lambda).
static int Consume(int aValue) { return aValue; }

template <typename F>
static int RunNow(F&& aFunc) {
  return aFunc();
}

int InitCapture(int aSeed) {
  return RunNow([seed = aSeed + 1]() { return Consume(seed); });
}

// A constructed object's value is what its constructor puts in it: by its
// definition (Box's), or for constructors and methods the analysis can't see
// into (Gecko's string classes', see strings.h), by their hints.
#include "strings.h"

class Box {
 public:
  explicit Box(int aValue) : mValue(aValue) {}
  int mValue;
};

static int Unbox(const Box& aBox) { return aBox.mValue; }

int Boxed(int aSeed) { return Unbox(Box(aSeed + 2)); }

static int UseName(const nsTSubstring<char>& aName) { return aName.Length(); }

int Names(const nsTSubstring<char>& aSource) {
  nsTString<char> copy(aSource);
  nsTString<char> assigned;
  assigned.Assign(aSource);
  return UseName(copy) + UseName(assigned) + UseName(nsTString<char>(aSource));
}
