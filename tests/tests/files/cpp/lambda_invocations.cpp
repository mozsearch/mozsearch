// Lambdas called right away or later by what they're passed to (see
// config_defaults/ontology-mapping.toml's "callable:sync" and
// "callable:async" labels, which these imitations of Gecko's functions have
// by their names, and crossref's `classify_lambdas`), for call diagrams,
// which fold those called right away into the functions making them, and
// draw those called later with "later" edges (see `fold_lambdas` in
// tools/src/cmd_pipeline/cmd_traverse.rs).

namespace mozilla {

// Like dom/quota's (which is in `mozilla`): calls its callables before it
// returns.
template <typename Cond, typename Body>
int CollectWhile(const Cond& aCond, const Body& aBody) {
  int n = 0;
  while (aCond()) {
    aBody();
    n++;
  }
  return n;
}

namespace dom {
namespace quota {

// Like dom/quota's: calls its callable through a lambda of its own, whose
// pretty name is its ("ExecuteInitialization::(lambda1)"), before it
// returns.
template <typename Func>
bool ExecuteInitialization(const Func& aFunc) {
  auto initialize = [&aFunc]() { return aFunc(); };
  return initialize();
}

}  // namespace quota
}  // namespace dom

// Like xpcom's: a promise whose callbacks' holders, partial specializations
// (whose pretty names have their template arguments:
// "MozPromise::ThenValue<type-parameter-1-0>::DoResolveOrRejectInternal"),
// call them later.
template <typename ResolveValueT>
class MozPromise {
 public:
  template <typename... Functions>
  class ThenValue;

  template <typename ResolveRejectFunction>
  class ThenValue<ResolveRejectFunction> {
   public:
    explicit ThenValue(const ResolveRejectFunction& aFunction)
        : mFunction(aFunction) {}
    virtual void DoResolveOrRejectInternal() { mFunction(); }

   private:
    ResolveRejectFunction mFunction;
  };
};

namespace detail {

// Like xpcom's: a runnable whose Run calls its function, later.
template <typename StoredFunction>
class RunnableFunction {
 public:
  explicit RunnableFunction(const StoredFunction& aFunction)
      : mFunction(aFunction) {}
  virtual void Run() { mFunction(); }

 private:
  StoredFunction mFunction;
};

}  // namespace detail
}  // namespace mozilla

namespace lambda_invocations {

bool More() { return false; }
void Step() {}
void Finish() {}
void Later() {}

void KeepFunction(const void* aFunction);

// Keeps what it's given (somewhere the analysis can't see it called).
template <typename F>
void Keep(const F& aF) {
  KeepFunction(&aF);
}

template <typename F>
mozilla::detail::RunnableFunction<F>* NewRunnable(const F& aF) {
  return new mozilla::detail::RunnableFunction<F>(aF);
}

// Calls its own lambda: right away.
void SelfCalled() {
  auto finish = []() { Finish(); };
  finish();
}

// Passes its lambdas to a function calling them right away.
void Collects() {
  mozilla::CollectWhile([]() { return More(); }, []() { Step(); });
}

// Passes its lambda to a function calling it right away, through a lambda of
// its own.
void Initializes() {
  mozilla::dom::quota::ExecuteInitialization([]() { return More(); });
}

// Passes its lambda to a runnable, which calls it later.
void Dispatches() {
  auto* runnable = NewRunnable([]() { Later(); });
  (void)runnable;
}

// Passes its lambda to a promise's callback holder, which calls it later.
void Thens() {
  auto later = []() { Later(); };
  auto* thenValue =
      new mozilla::MozPromise<int>::ThenValue<decltype(later)>(later);
  (void)thenValue;
}

// Passes its lambda on to a function nothing indexed says calls it.
void Unknown() {
  Keep([]() { Step(); });
}

}  // namespace lambda_invocations
