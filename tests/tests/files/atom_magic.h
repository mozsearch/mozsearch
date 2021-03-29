/* This file tries to approximate nsGkAtoms.h by operating in conjunction with
   `atom_list.h` while also providing an example of a header file that's
   included multiple times.  (We didn't really have coverage for this before,
   and this is important for our merging logic.) */

#ifndef atom_magic_h___
#define atom_magic_h___

struct YoAtoms {
#define YO_ATOM(name_, value_) \
  const char16_t name_##_string[sizeof(value_)];
#include "atom_list.h"
#undef YO_ATOM

  enum class Atoms {
#define YO_ATOM(name_, value_) name_,
#include "atom_list.h"
#undef YO_ATOM
    AtomsCount
  };
};

#endif /* atom_magic_h__ */
