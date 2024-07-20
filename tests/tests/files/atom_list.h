// This file is attempting to imitate nsGkAtomList.h and is intended to be
// included by `atom_magic.h`

YO_ATOM(Foo, "foo")
YO_ATOM(Bar, "bar")

#define NESTED_YO_ATOM(A, B) YO_ATOM(A, B)

NESTED_YO_ATOM(Baz, "baz")
