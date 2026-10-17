// Fixtures for parameters' unified symbols (see
// tools/src/file_format/parameter_symbols.rs): a declaration here and the
// definition in params.cpp share their parameters' symbols.

namespace params {

int Scale(int aValue, int aFactor);

// An unnamed parameter: the named one after it keeps its own index, which
// its type says (an int can't be the second parameter).
int Second(int, bool aSecond);

// An unnamed parameter whose named neighbor's type doesn't say which it is:
// the declaration's keeps its own symbol.
int Third(int, int aThird);

}  // namespace params
