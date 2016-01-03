namespace NS {

struct R;

struct R {
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

struct S2 {
    virtual void v() = 0;
};

struct T : public S, public S2 {
    virtual void v();
    void m();
    void m(int);
};

void f() {}
void g();

template<typename T>
class X {
  public:
    X() {}

    void f();
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

int main()
{
    NS::f();
    NS::g();
    NS::S s;
    s.m();
    HELLO(4);

    void (*fp)();
    fp = &NS::f;
    fp();

    NS::S* sp = new NS::S();

    NS::X<char> xx;
    xx.f();

    NS::X<int> xy;
    xy.f();

    NS::templateFunc(47);

    NS::templateFunc('c');

    NS::Dummy::Hello();

    return 0;
}
