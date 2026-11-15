// Lambdas called right away or later by what they're passed to (see
// config_defaults/ontology-mapping.toml's "callable:sync" and
// "callable:async" labels, which these imitations of Gecko's functions have
// by their names, and crossref's `classify_lambdas`), for call diagrams,
// which fold those called right away into the functions making them, and
// draw those called later with "later" edges (see `fold_lambdas` in
// tools/src/cmd_pipeline/cmd_traverse.rs).

namespace mozilla {
namespace dom {
namespace quota {

// Like dom/quota's: calls its callables before it returns.
template <typename Cond, typename Body>
int CollectWhile(const Cond& aCond, const Body& aBody) {
  int n = 0;
  while (aCond()) {
    aBody();
    n++;
  }
  return n;
}

}  // namespace quota
}  // namespace dom

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
  mozilla::dom::quota::CollectWhile([]() { return More(); },
                                    []() { Step(); });
}

// Passes its lambda to a runnable, which calls it later.
void Dispatches() {
  auto* runnable = NewRunnable([]() { Later(); });
  (void)runnable;
}

// Passes its lambda on to a function nothing indexed says calls it.
void Unknown() {
  Keep([]() { Step(); });
}

}  // namespace lambda_invocations
