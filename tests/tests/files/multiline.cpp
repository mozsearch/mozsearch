#include "atom_magic.h"

/**
 * Some documentation.
 */
void
f(int a,
  int b,
  int c)
{
  return;
}

struct Forward;

class B {};
class C {};

/* Class is is some
 * thing that does stuff.
 */
class A
  : public B,
    private C
{
  /*
   * This is field.
   */
  int field;

  // other field;
  int other;

  friend class B;
};
