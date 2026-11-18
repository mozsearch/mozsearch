// Fixtures for roll-ups (see tools/src/file_format/rollup.rs) and dataflow
// summaries (see tools/src/dataflow/summary.rs): a model class here, in the
// "Core/Rollup Model" subsystem, and view classes in view.cpp, in "Core/Rollup
// View", which call it and move data through it.

namespace rollup {

// (Held by value: the store's own state, so its writes aren't data moving
// to another class.)
struct Stats {
  int mLast = 0;
};

class Store {
 public:
  void Put(int aValue);
  int Get() const { return mValue; }
  // (Plumbing that roll-ups leave out.)
  void AssertValid() const {}

 private:
  int mValue = 0;
  Stats mStats;
};

}  // namespace rollup

namespace mozilla {

// (A function of one of diagram-mapping.toml's groups of left-out calls, the
// time's, whose calls roll-ups leave out too: Panel's call of Now() isn't in
// its roll-up.)
class TimeStamp {
 public:
  static TimeStamp Now() { return TimeStamp(); }
};

}  // namespace mozilla
