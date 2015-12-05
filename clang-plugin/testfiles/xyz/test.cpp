namespace NS {

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

}

int main()
{
    NS::f();
    NS::g();
    NS::S s;
    s.m();
    s.m(4);

    return 0;
}
