//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_utils.idl
//


/// `interface nsIXPCTestFunctionInterface : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestFunctionInterface {
    vtable: &'static nsIXPCTestFunctionInterfaceVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads by default, as
    /// XPCOM is generally not threadsafe.
    ///
    /// If this type is marked as [rust_sync], there will be explicit `Send` and
    /// `Sync` implementations on this type, which will override the inherited
    /// negative impls from `Rc`.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,

    // Make the rust compiler aware that there might be interior mutability
    // in what actually implements the interface. This works around UB
    // introduced by https://github.com/llvm/llvm-project/commit/01859da84bad95fd51d6a03b08b60c660e642a4f
    // that a rust lint would make blatantly obvious, but doesn't exist.
    // (See https://github.com/rust-lang/rust/issues/111229).
    // This prevents optimizations, but those optimizations weren't available
    // before rustc switched to LLVM 16, and they now cause problems because
    // of the UB.
    // Until there's a lint available to find all our UB, it's simpler to
    // avoid the UB in the first place, at the cost of preventing optimizations
    // in places that don't cause UB. But again, those optimizations weren't
    // available before.
    __maybe_interior_mutability: ::std::cell::UnsafeCell<[u8; 0]>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestFunctionInterface.
unsafe impl XpCom for nsIXPCTestFunctionInterface {
    const IID: nsIID = nsID(0xd58a82ab, 0xd8f7, 0x4ca9,
        [0x92, 0x73, 0xb3, 0x29, 0x0d, 0x42, 0xa0, 0xcf]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestFunctionInterface {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestFunctionInterface.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestFunctionInterfaceCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestFunctionInterface`.
    fn coerce_from(v: &nsIXPCTestFunctionInterface) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestFunctionInterfaceCoerce for nsIXPCTestFunctionInterface {
    #[inline]
    fn coerce_from(v: &nsIXPCTestFunctionInterface) -> &Self {
        v
    }
}

impl nsIXPCTestFunctionInterface {
    /// Cast this `nsIXPCTestFunctionInterface` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestFunctionInterfaceCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestFunctionInterface {
    type Target = nsISupports;
    #[inline]
    fn deref(&self) -> &nsISupports {
        unsafe {
            ::std::mem::transmute(self)
        }
    }
}

// Ensure we can use .coerce() to cast to our base types as well. Any type which
// our base interface can coerce from should be coercable from us as well.
impl<T: nsISupportsCoerce> nsIXPCTestFunctionInterfaceCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestFunctionInterface) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestFunctionInterface
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestFunctionInterfaceVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* string echo (in string arg); */
    pub Echo: unsafe extern "system" fn (this: *const nsIXPCTestFunctionInterface, arg: *const libc::c_char, _retval: *mut *const libc::c_char) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestFunctionInterface {


    /// `string echo (in string arg);`
    #[inline]
    pub unsafe fn Echo(&self, arg: *const libc::c_char, _retval: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).Echo)(self, arg, _retval)
    }


}


/// `interface nsIXPCTestUtils : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestUtils {
    vtable: &'static nsIXPCTestUtilsVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads by default, as
    /// XPCOM is generally not threadsafe.
    ///
    /// If this type is marked as [rust_sync], there will be explicit `Send` and
    /// `Sync` implementations on this type, which will override the inherited
    /// negative impls from `Rc`.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,

    // Make the rust compiler aware that there might be interior mutability
    // in what actually implements the interface. This works around UB
    // introduced by https://github.com/llvm/llvm-project/commit/01859da84bad95fd51d6a03b08b60c660e642a4f
    // that a rust lint would make blatantly obvious, but doesn't exist.
    // (See https://github.com/rust-lang/rust/issues/111229).
    // This prevents optimizations, but those optimizations weren't available
    // before rustc switched to LLVM 16, and they now cause problems because
    // of the UB.
    // Until there's a lint available to find all our UB, it's simpler to
    // avoid the UB in the first place, at the cost of preventing optimizations
    // in places that don't cause UB. But again, those optimizations weren't
    // available before.
    __maybe_interior_mutability: ::std::cell::UnsafeCell<[u8; 0]>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestUtils.
unsafe impl XpCom for nsIXPCTestUtils {
    const IID: nsIID = nsID(0x1e9cddeb, 0x510d, 0x449a,
        [0xb1, 0x52, 0x3c, 0x1b, 0x5b, 0x31, 0xd4, 0x1d]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestUtils {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestUtils.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestUtilsCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestUtils`.
    fn coerce_from(v: &nsIXPCTestUtils) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestUtilsCoerce for nsIXPCTestUtils {
    #[inline]
    fn coerce_from(v: &nsIXPCTestUtils) -> &Self {
        v
    }
}

impl nsIXPCTestUtils {
    /// Cast this `nsIXPCTestUtils` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestUtilsCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestUtils {
    type Target = nsISupports;
    #[inline]
    fn deref(&self) -> &nsISupports {
        unsafe {
            ::std::mem::transmute(self)
        }
    }
}

// Ensure we can use .coerce() to cast to our base types as well. Any type which
// our base interface can coerce from should be coercable from us as well.
impl<T: nsISupportsCoerce> nsIXPCTestUtilsCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestUtils) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestUtils
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestUtilsVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f); */
    pub DoubleWrapFunction: unsafe extern "system" fn (this: *const nsIXPCTestUtils, f: *const nsIXPCTestFunctionInterface, _retval: *mut *const nsIXPCTestFunctionInterface) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestUtils {


    /// `nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f);`
    #[inline]
    pub unsafe fn DoubleWrapFunction(&self, f: *const nsIXPCTestFunctionInterface, _retval: *mut *const nsIXPCTestFunctionInterface) -> ::nserror::nsresult {
        ((*self.vtable).DoubleWrapFunction)(self, f, _retval)
    }


}


/// `typedef void *  Noncompat;`
///

/// ```text
/// /**
///  * TypeScript bindings specific tests.
///  */
/// ```
///

pub type Noncompat = *mut libc::c_void;


/// `interface nsIXPCTestNotScriptable : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestNotScriptable {
    vtable: &'static nsIXPCTestNotScriptableVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads by default, as
    /// XPCOM is generally not threadsafe.
    ///
    /// If this type is marked as [rust_sync], there will be explicit `Send` and
    /// `Sync` implementations on this type, which will override the inherited
    /// negative impls from `Rc`.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,

    // Make the rust compiler aware that there might be interior mutability
    // in what actually implements the interface. This works around UB
    // introduced by https://github.com/llvm/llvm-project/commit/01859da84bad95fd51d6a03b08b60c660e642a4f
    // that a rust lint would make blatantly obvious, but doesn't exist.
    // (See https://github.com/rust-lang/rust/issues/111229).
    // This prevents optimizations, but those optimizations weren't available
    // before rustc switched to LLVM 16, and they now cause problems because
    // of the UB.
    // Until there's a lint available to find all our UB, it's simpler to
    // avoid the UB in the first place, at the cost of preventing optimizations
    // in places that don't cause UB. But again, those optimizations weren't
    // available before.
    __maybe_interior_mutability: ::std::cell::UnsafeCell<[u8; 0]>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestNotScriptable.
unsafe impl XpCom for nsIXPCTestNotScriptable {
    const IID: nsIID = nsID(0xddf64cfb, 0x668a, 0x4571,
        [0xa9, 0x00, 0x0f, 0xe2, 0xba, 0xbb, 0x62, 0x49]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestNotScriptable {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestNotScriptable.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestNotScriptableCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestNotScriptable`.
    fn coerce_from(v: &nsIXPCTestNotScriptable) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestNotScriptableCoerce for nsIXPCTestNotScriptable {
    #[inline]
    fn coerce_from(v: &nsIXPCTestNotScriptable) -> &Self {
        v
    }
}

impl nsIXPCTestNotScriptable {
    /// Cast this `nsIXPCTestNotScriptable` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestNotScriptableCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestNotScriptable {
    type Target = nsISupports;
    #[inline]
    fn deref(&self) -> &nsISupports {
        unsafe {
            ::std::mem::transmute(self)
        }
    }
}

// Ensure we can use .coerce() to cast to our base types as well. Any type which
// our base interface can coerce from should be coercable from us as well.
impl<T: nsISupportsCoerce> nsIXPCTestNotScriptableCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestNotScriptable) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestNotScriptable
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestNotScriptableVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestNotScriptable {


}


/// `interface nsIXPCTestTypeScript : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestTypeScript {
    vtable: &'static nsIXPCTestTypeScriptVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads by default, as
    /// XPCOM is generally not threadsafe.
    ///
    /// If this type is marked as [rust_sync], there will be explicit `Send` and
    /// `Sync` implementations on this type, which will override the inherited
    /// negative impls from `Rc`.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,

    // Make the rust compiler aware that there might be interior mutability
    // in what actually implements the interface. This works around UB
    // introduced by https://github.com/llvm/llvm-project/commit/01859da84bad95fd51d6a03b08b60c660e642a4f
    // that a rust lint would make blatantly obvious, but doesn't exist.
    // (See https://github.com/rust-lang/rust/issues/111229).
    // This prevents optimizations, but those optimizations weren't available
    // before rustc switched to LLVM 16, and they now cause problems because
    // of the UB.
    // Until there's a lint available to find all our UB, it's simpler to
    // avoid the UB in the first place, at the cost of preventing optimizations
    // in places that don't cause UB. But again, those optimizations weren't
    // available before.
    __maybe_interior_mutability: ::std::cell::UnsafeCell<[u8; 0]>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestTypeScript.
unsafe impl XpCom for nsIXPCTestTypeScript {
    const IID: nsIID = nsID(0x1bbfe703, 0xc67d, 0x4995,
        [0xb0, 0x61, 0x56, 0x4c, 0x8a, 0x1c, 0x39, 0xd7]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestTypeScript {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestTypeScript.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestTypeScriptCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestTypeScript`.
    fn coerce_from(v: &nsIXPCTestTypeScript) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestTypeScriptCoerce for nsIXPCTestTypeScript {
    #[inline]
    fn coerce_from(v: &nsIXPCTestTypeScript) -> &Self {
        v
    }
}

impl nsIXPCTestTypeScript {
    /// Cast this `nsIXPCTestTypeScript` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestTypeScriptCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestTypeScript {
    type Target = nsISupports;
    #[inline]
    fn deref(&self) -> &nsISupports {
        unsafe {
            ::std::mem::transmute(self)
        }
    }
}

// Ensure we can use .coerce() to cast to our base types as well. Any type which
// our base interface can coerce from should be coercable from us as well.
impl<T: nsISupportsCoerce> nsIXPCTestTypeScriptCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestTypeScript) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestTypeScript
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestTypeScriptVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* attribute long exposedProp; */
    pub GetExposedProp: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, aExposedProp: *mut i32) -> ::nserror::nsresult,

    /* attribute long exposedProp; */
    pub SetExposedProp: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, aExposedProp: i32) -> ::nserror::nsresult,

    /* void exposedMethod (in long arg); */
    pub ExposedMethod: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, arg: i32) -> ::nserror::nsresult,

    /* [noscript] attribute Noncompat noncompatProp; */
    pub GetNoncompatProp: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, aNoncompatProp: *mut Noncompat) -> ::nserror::nsresult,

    /* [noscript] attribute Noncompat noncompatProp; */
    pub SetNoncompatProp: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, aNoncompatProp: Noncompat) -> ::nserror::nsresult,

    /* [noscript] void noncompatMethod (in Noncompat arg); */
    pub NoncompatMethod: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, arg: Noncompat) -> ::nserror::nsresult,

    /* [noscript] attribute long noscriptProp; */
    pub GetNoscriptProp: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, aNoscriptProp: *mut i32) -> ::nserror::nsresult,

    /* [noscript] attribute long noscriptProp; */
    pub SetNoscriptProp: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, aNoscriptProp: i32) -> ::nserror::nsresult,

    /* [noscript] void noscriptMethod (in long arg); */
    pub NoscriptMethod: unsafe extern "system" fn (this: *const nsIXPCTestTypeScript, arg: i32) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestTypeScript {


    /// `attribute long exposedProp;`
    #[inline]
    pub unsafe fn GetExposedProp(&self, aExposedProp: *mut i32) -> ::nserror::nsresult {
        ((*self.vtable).GetExposedProp)(self, aExposedProp)
    }



    /// `attribute long exposedProp;`
    #[inline]
    pub unsafe fn SetExposedProp(&self, aExposedProp: i32) -> ::nserror::nsresult {
        ((*self.vtable).SetExposedProp)(self, aExposedProp)
    }



    /// `void exposedMethod (in long arg);`
    #[inline]
    pub unsafe fn ExposedMethod(&self, arg: i32) -> ::nserror::nsresult {
        ((*self.vtable).ExposedMethod)(self, arg)
    }



    /// `[noscript] attribute Noncompat noncompatProp;`
    #[inline]
    pub unsafe fn GetNoncompatProp(&self, aNoncompatProp: *mut Noncompat) -> ::nserror::nsresult {
        ((*self.vtable).GetNoncompatProp)(self, aNoncompatProp)
    }



    /// `[noscript] attribute Noncompat noncompatProp;`
    #[inline]
    pub unsafe fn SetNoncompatProp(&self, aNoncompatProp: Noncompat) -> ::nserror::nsresult {
        ((*self.vtable).SetNoncompatProp)(self, aNoncompatProp)
    }



    /// `[noscript] void noncompatMethod (in Noncompat arg);`
    #[inline]
    pub unsafe fn NoncompatMethod(&self, arg: Noncompat) -> ::nserror::nsresult {
        ((*self.vtable).NoncompatMethod)(self, arg)
    }



    /// `[noscript] attribute long noscriptProp;`
    #[inline]
    pub unsafe fn GetNoscriptProp(&self, aNoscriptProp: *mut i32) -> ::nserror::nsresult {
        ((*self.vtable).GetNoscriptProp)(self, aNoscriptProp)
    }



    /// `[noscript] attribute long noscriptProp;`
    #[inline]
    pub unsafe fn SetNoscriptProp(&self, aNoscriptProp: i32) -> ::nserror::nsresult {
        ((*self.vtable).SetNoscriptProp)(self, aNoscriptProp)
    }



    /// `[noscript] void noscriptMethod (in long arg);`
    #[inline]
    pub unsafe fn NoscriptMethod(&self, arg: i32) -> ::nserror::nsresult {
        ((*self.vtable).NoscriptMethod)(self, arg)
    }


}


