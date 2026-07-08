/** <!-- binding_to(idl, class, XPIDL_nsIXPCTestObjectReadWrite) --> */
declare class tsClass  {
    /** <!-- binding_to(idl, attribute, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) --> */
    tsAttribute: string;

    /** <!-- binding_to(idl, getter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) --> */
    tsGetter(): string;

    /** <!-- binding_to(idl, setter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) --> */
    tsSetter(value: string);
}

/** <!-- binding_to(idl, method, XPIDL_nsIXPCTestParams_testBoolean) --> */
declare function tsMethod(a: boolean, b: boolean): boolean;

/** <!-- binding_to(cpp, class, T_NS::R::XYZ) --> */
enum TsEnum {
    /** <!-- binding_to(cpp, const, E_<T_NS::R::XYZ>_TAG1) --> */
    TsConstInbound,
    /** <!-- bound_as(cpp, const, E_<T_NS::R::XYZ>_TAG2) --> */
    TsConstOutbound,
}
