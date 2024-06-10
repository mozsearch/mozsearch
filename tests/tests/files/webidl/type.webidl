interface TestInterfaceForType {
  void Func1(NullableType? arg1,
             sequence<ItemType1> arg2,
             record<DOMString, ValueType> arg3,
             ObservableArray<ItemType2> arg4,
             Promise<PromiseValueType> arg5,
             (UnionItemType1 or UnionItemType2 or UnionItemType3) arg5,
             unsigned long arg6);
};
