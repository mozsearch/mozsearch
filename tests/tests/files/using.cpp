#include <stdint.h>

template<typename T>
using MyType = T;

template<class T>
class TemplatedClass
{
    using SomeType = MyType<T>;

    int foo() {
        SomeType x;
        return x;
    }
};

MyType<int32_t> xyz;
TemplatedClass<int> abc;
