template<typename T>
class X { X(int); };
// This used to trigger UB, see bug 2088068
X(int) -> X<int>;
