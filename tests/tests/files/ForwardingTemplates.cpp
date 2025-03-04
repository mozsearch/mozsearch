#include <memory>
#include <vector>

struct StructUsedInTypeDependentNew0 {
  StructUsedInTypeDependentNew0() {}
};

struct StructUsedInTypeDependentNew1 {
  StructUsedInTypeDependentNew1() {}
};

struct StructUsedInTypeIndependentNew {
  StructUsedInTypeIndependentNew() {}
};

template <typename T, typename... Args>
std::unique_ptr<T> MakeUniqueWithIndex(int i, Args&&... args) {
  const auto _ = std::unique_ptr<StructUsedInTypeIndependentNew>{
      new StructUsedInTypeIndependentNew()};
  return std::unique_ptr<T>{new T(std::forward<Args>(args)...)};
}

template <typename T, typename... Args>
std::unique_ptr<T> MakeUnique(Args&&... args) {
  const auto _ = std::unique_ptr<StructUsedInTypeIndependentNew>{
      new StructUsedInTypeIndependentNew()};
  return MakeUniqueWithIndex<T>(0, std::forward<Args>(args)...);
}

template <typename T, typename... Args>
std::unique_ptr<T> RecursiveMakeUnique(Args&&... args) {
  const auto _ = RecursiveMakeUnique<T>(std::forward<Args>(args)...);
  return MakeUnique<T>(std::forward<Args>(args)...);
}

template <typename T, typename... Args>
std::unique_ptr<T> MakeUniqueWithLambda(Args&&... args) {
  return std::unique_ptr<T>{[t = new T()] { return t; }()};
}

void test() {
  const auto a = MakeUniqueWithIndex<StructUsedInTypeDependentNew0>(0);
  const auto b = MakeUniqueWithIndex<StructUsedInTypeDependentNew0>(0);
  const auto c = MakeUniqueWithIndex<StructUsedInTypeDependentNew1>(0);
  const auto d = MakeUniqueWithIndex<StructUsedInTypeDependentNew1>(0);
  const auto e = MakeUnique<StructUsedInTypeDependentNew0>();
  const auto f = MakeUnique<StructUsedInTypeDependentNew0>();
  const auto g = MakeUnique<StructUsedInTypeDependentNew1>();
  const auto h = MakeUnique<StructUsedInTypeDependentNew1>();
  const auto i = RecursiveMakeUnique<StructUsedInTypeDependentNew0>();
  const auto j = RecursiveMakeUnique<StructUsedInTypeDependentNew0>();
  const auto k = RecursiveMakeUnique<StructUsedInTypeDependentNew1>();
  const auto l = RecursiveMakeUnique<StructUsedInTypeDependentNew1>();
  const auto m = MakeUniqueWithLambda<StructUsedInTypeDependentNew0>();
  const auto n = MakeUniqueWithLambda<StructUsedInTypeDependentNew0>();
  const auto o = MakeUniqueWithLambda<StructUsedInTypeDependentNew1>();
  const auto p = MakeUniqueWithLambda<StructUsedInTypeDependentNew1>();

  const auto stl = std::make_unique<StructUsedInTypeDependentNew0>();
}

template <typename T>
struct Maybe;

template <typename T>
struct Maybe {
  char storage[sizeof(T)];

  template <typename... Args>
  void emplace_inline(Args&&... args) {
    new (storage) T(std::forward<Args>(args)...);
  }

  template <typename... Args>
  void emplace_out_of_line(Args&&... args);
};

template <typename T>
template <typename... Args>
void Maybe<T>::emplace_out_of_line(Args&&... args) {
  new (storage) T(std::forward<Args>(args)...);
}

struct StructUsedInEmplace {
  StructUsedInEmplace() {}
};

void use_maybe() {
  Maybe<StructUsedInEmplace> m;
  m.emplace_inline();
  m.emplace_out_of_line();

  std::vector<StructUsedInEmplace> v;
  v.emplace_back();
}
