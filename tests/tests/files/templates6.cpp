#include "templates6.h"

template <typename T> void multiplexer(T t) { overloaded(t); }

int main() {
  multiplexer(1);
  multiplexer('a');
}
