#include <stdio.h>
#include <stdlib.h>

class Dispatcher;

class nsIRunnable {
 public:
  virtual void Run(Dispatcher* aDispatcher) = 0;
};

void common_shoe_related_method(void) { printf("shoes do things\n"); }

class Dispatcher {
 public:
  void Dispatch(nsIRunnable* aRunnable) {
    // eh, we don't do anything right now.
  }
};

class ShoelaceRunnable : public nsIRunnable {
 public:
  ShoelaceRunnable() = default;

  void Run(Dispatcher* aDispatcher) override {
    printf("I am a shoelace!\n");
    common_shoe_related_method();
  }
};

class ShoeRunnable : public nsIRunnable {
 public:
  ShoeRunnable() = default;

  void Run(Dispatcher* aDispatcher) override {
    printf("I am a shoe that runs!\n");
    ShoelaceRunnable shoelace;
    aDispatcher->Dispatch(&shoelace);
  }
};

class SandalRunnable : public ShoeRunnable {
 public:
  SandalRunnable() = default;

  void Run(Dispatcher* aDispatcher) override {
    printf("I am a shoe that is a sandal that runs!\n");
    common_shoe_related_method();
  }
};

int main(void) {
  Dispatcher d;

  ShoeRunnable shoe;
  SandalRunnable sandal;
  d.Dispatch(&shoe);
  d.Dispatch(&sandal);
}
