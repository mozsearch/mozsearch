#include <cstddef>
#include <cstdio>

constexpr auto operator""_argggh(const char* aStr, std::size_t aLen) {
  return aStr;
}

// This can't be named main() or we'll mess up an existing graph test, whoops.
int use_the_argggh_operator() {
  const char* blah = "blah"_argggh;
  printf("%s\n", blah);

  return 0;
}
