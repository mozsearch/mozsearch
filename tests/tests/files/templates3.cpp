class Foo {};
class Baz {};

template<class T>
class Traits
{
  public:
    static void Method() {}
};

template<>
class Traits<Foo>
{
  public:
    static void Method() {}
};

class Bar
{
  public:
    template<class T>
    void
    Function(T* t);
};

template<class T>
inline void
Bar::Function(T* t)
{
    Traits<T>::Method();
}

int
main()
{
    Foo* f;
    Baz* z;
    Bar* b;
    b->Function(f);
    b->Function(z);
    return 0;
}
