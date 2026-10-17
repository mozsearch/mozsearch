// See model.h: a controller passes its input to a panel, which stores it in
// the model and shows what the model has.

#include "rollup/model.h"

namespace rollup {

class Panel {
 public:
  explicit Panel(Store* aStore) : mStore(aStore) {}

  void Refresh() {
    mStore->AssertValid();
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
