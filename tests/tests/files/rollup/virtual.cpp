// Fixtures for composing dataflow summaries through virtual calls (see
// tools/src/dataflow/summary.rs's `field_edges`): an emitter's field reaches a
// recorder's through the listener interface the recorder implements.

namespace rollup {

class Listener {
 public:
  virtual void OnValue(int aValue) = 0;
};

class Recorder : public Listener {
 public:
  void OnValue(int aValue) override { mLast = aValue; }

 private:
  int mLast = 0;
};

class Emitter {
 public:
  explicit Emitter(Listener* aListener) : mListener(aListener) {}
  void Emit() { mListener->OnValue(mCurrent); }

 private:
  Listener* mListener;
  int mCurrent = 3;
};

void UseVirtual() {
  Recorder recorder;
  Emitter emitter(&recorder);
  emitter.Emit();
}

}  // namespace rollup
