// Fixtures for roll-ups (see tools/src/file_format/rollup.rs) and dataflow
// summaries (see tools/src/dataflow/summary.rs): a model class here, in the
// "Core/Rollup Model" subsystem, and view classes in view.cpp, in "Core/Rollup
// View", which call it and move data through it.

namespace rollup {

class Store {
 public:
  void Put(int aValue);
  int Get() const { return mValue; }
  // (Plumbing that roll-ups leave out.)
  void AssertValid() const {}

 private:
  int mValue = 0;
};

}  // namespace rollup
