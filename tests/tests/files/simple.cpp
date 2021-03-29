#include <stdio.h>

#include "atom_magic.h"

extern "C" void WithNoMangle();

extern "C" void ExternFunctionImplementedInCpp() { }

namespace NS {

struct R;

enum {
    TAG3
};

typedef struct {
    int f;

    bool operator()(int);
} Abc;

struct R {
    enum XYZ {
	TAG1,
	TAG2
    };

    virtual void v() = 0;
};

struct S : public R
{
    S();
    ~S();
    void m();
    void m(int);
    virtual void v();
};

namespace {
int xyz;
};

struct S2 {
    virtual void v() = 0;
};

struct T : public S, public S2 {
    virtual void v();
    void m();
    void m(int);

    int field;
};

template<typename T>
struct OtherObj {
    OtherObj(char c) {}
};

template<typename T>
struct StackObj {
    StackObj(int x) : mOther('x') {}

    OtherObj<T> mOther;
};

void f() {}
void g();

int cxx14DigitSeparators() {
    return 0b1100'1111;
}

typedef R OtherR;

template<typename T>
class X {
  public:
    X() {}

    void f();

    int field;
};

template<typename T>
void X<T>::f() {}

template<>
void X<int>::f() {}

template<typename T>
void templateFunc(const T& arg);

template<>
void templateFunc(const char& arg);

struct Dummy {
#define DECL_SOMETHING(Env, Name) \
    static bool Name() {	  \
	return Env;		  \
    }

    DECL_SOMETHING(true, Hello);
    DECL_SOMETHING(false, Goodbye);
};
}

#define HELLO s.m

class Q {
    typedef int (Q::*AddressReader)(const char*) const;
};

extern int GLOBAL;

int main()
{
    GLOBAL = NS::TAG3;

    NS::OtherR* otherr;

    NS::f();
    NS::g();
    NS::S s;
    s.m();
    HELLO(4);

#ifdef HELLO
    int abc;
#endif

#if defined(HELLO)
    int abc1;
#endif

#undef HELLO

    void (*fp)();
    fp = &NS::f;
    fp();

    NS::S* sp = new NS::S();

    NS::X<char> xx;
    xx.f();
    xx.field = 12;

    NS::X<int> xy;
    xy.f();

    NS::templateFunc(47);

    NS::templateFunc('c');

    NS::Dummy::Hello();

    NS::StackObj<int> stackobj(10);

    WithNoMangle();

    return 0;
}
