#define NS_GENERIC_FACTORY_CONSTRUCTOR_INIT(_InstanceClass, _InitMethod) \
static _InstanceClass*                                                  \
_InstanceClass##Constructor()                                           \
{                                                                       \
  _InstanceClass* inst = new _InstanceClass();                          \
  inst->_InitMethod();                                                  \
  return inst;                                                          \
}


class NullPrincipal {
public:
  void Init() {
  }
};

NS_GENERIC_FACTORY_CONSTRUCTOR_INIT(NullPrincipal, Init)

int main() {
    NullPrincipal* p = NullPrincipalConstructor();
    delete p;
    return 0;
}
