// Accessors, which traces step over by their dataflow summaries rather than
// building their files' IR (see tools/src/dataflow/trace.rs's
// `accessor_summary`), and which accessors.cpp calls.

namespace accessors {

class Gauge {
 public:
  int Level() const { return mLevel; }
  void SetLevel(int aLevel) { mLevel = aLevel; }
  // (A constant's write, which summaries' flows leave out, but their
  // constants have.)
  void Reset() { mLevel = 0; }
  bool IsReady() const { return true; }

 private:
  int mLevel = 0;
};

}  // namespace accessors
