// See model.h: a controller passes its input to a panel, which stores it in
// the model and shows what the model has.

#include "rollup/model.h"

namespace rollup {

class Panel {
 public:
  explicit Panel(Store* aStore) : mStore(aStore) {}

  void Refresh() {
    mStore->AssertValid();
    mozilla::TimeStamp::Now();
    mShown = mStore->Get();
  }

  void Edit(int aValue) {
    mStore->Put(aValue);
    Refresh();
  }

 private:
  Store* mStore;
  int mShown = 0;
};

class Controller {
 public:
  void Run(Panel& aPanel) {
    aPanel.Edit(mInput);
    aPanel.Refresh();
  }

 private:
  int mInput = 7;
};

void UseThem() {
  Store store;
  Panel panel(&store);
  Controller controller;
  controller.Run(panel);
}

}  // namespace rollup

// (A serializer, as IPC's ParamTraits specializations are, of the store's
// stats: its reads and writes of them are how they're sent, not what the code
// does, which roll-ups leave out, as diagram-mapping.toml's `ipc` group's
// calls, so that IPC::ParamTraits, which all of its specializations are, isn't
// one of Rollup View's classes, with flows to and from Store.)
namespace IPC {

template <class P>
struct ParamTraits;

template <>
struct ParamTraits<rollup::Stats> {
  static void Write(int* aWriter, const rollup::Stats& aParam) {
    *aWriter = aParam.mLast;
  }
  static bool Read(const int* aReader, rollup::Stats* aResult) {
    aResult->mLast = *aReader;
    return true;
  }
};

}  // namespace IPC
