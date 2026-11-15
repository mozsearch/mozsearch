// Calls that calls diagrams leave out (see config_defaults/diagram-mapping.toml
// and `LeftOutCalls` in tools/src/cmd_pipeline/cmd_traverse.rs): those of
// these imitations of Gecko's infrastructure, by their names (mfbt's
// assertions, UniquePtr and Maybe, and the strings' `_ns` literals), and
// those of getters, by their dataflow summaries.

static void MOZ_ReportAssertionFailure(const char* aStr, const char* aFilename,
                                       int aLine) {}
static void AnnotateMozCrashReason(const char* aReason) {}

// (As debug builds expand it.)
#define MOZ_ASSERT(expr)                                    \
  do {                                                      \
    if (!(expr)) {                                          \
      AnnotateMozCrashReason("MOZ_ASSERT(" #expr ")");      \
      MOZ_ReportAssertionFailure(#expr, __FILE__, __LINE__); \
    }                                                       \
  } while (false)

constexpr decltype(sizeof(0)) operator""_ns(const char* aStr,
                                            decltype(sizeof(0)) aLength) {
  return aLength;
}

namespace mozilla {

template <typename T>
class UniquePtr {
 public:
  explicit UniquePtr(T* aPtr) : mPtr(aPtr) {}
  ~UniquePtr() { reset(); }
  T* operator->() const { return mPtr; }
  T* get() const { return mPtr; }
  // (Calls `get`, which its diagram shows: it's of a smart pointer.)
  void reset() {
    delete get();
    mPtr = nullptr;
  }

 private:
  T* mPtr;
};

template <typename T>
class Maybe {
 public:
  Maybe() : mIsSome(false) {}
  bool isSome() const { return mIsSome; }

 private:
  bool mIsSome;
};

}  // namespace mozilla

namespace left_out_calls {

void Step() {}

class Thing {
 public:
  // A getter: it only returns a field, and calls nothing.
  int Value() const { return mValue; }
  void Poke() { Step(); }

 private:
  int mValue = 0;
};

// Calls the code's logic (Thing::Poke, Step) and infrastructure, which its
// diagram leaves out.
int Works(mozilla::UniquePtr<Thing>& aThing, const mozilla::Maybe<int>& aMaybe) {
  MOZ_ASSERT(aMaybe.isSome());
  aThing->Poke();
  Step();
  return aThing->Value() + "work"_ns;
}

}  // namespace left_out_calls
