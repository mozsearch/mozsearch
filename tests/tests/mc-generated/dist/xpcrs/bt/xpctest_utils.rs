//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_utils.idl
//


{static D: &[Interface] = &[

        Interface {
            name: "nsIXPCTestFunctionInterface",
            base: Some("nsISupports"),
            sync: false,
            methods: Ok(&[
                    /* string echo (in string arg); */
                    Method {
                        name: "Echo",
                        params: &[Param { name: "arg", ty: "*const libc::c_char" }, Param { name: "_retval", ty: "*mut *const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        Interface {
            name: "nsIXPCTestUtils",
            base: Some("nsISupports"),
            sync: false,
            methods: Ok(&[
                    /* nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f); */
                    Method {
                        name: "DoubleWrapFunction",
                        params: &[Param { name: "f", ty: "*const nsIXPCTestFunctionInterface" }, Param { name: "_retval", ty: "*mut *const nsIXPCTestFunctionInterface" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        Interface {
            name: "nsIXPCTestNotScriptable",
            base: Some("nsISupports"),
            sync: false,
            methods: Ok(&[
                    ]),
        },

        Interface {
            name: "nsIXPCTestTypeScript",
            base: Some("nsISupports"),
            sync: false,
            methods: Ok(&[
                    /* attribute long exposedProp; */
                    Method {
                        name: "GetExposedProp",
                        params: &[Param { name: "aExposedProp", ty: "*mut i32" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetExposedProp",
                        params: &[Param { name: "aExposedProp", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    /* void exposedMethod (in long arg); */
                    Method {
                        name: "ExposedMethod",
                        params: &[Param { name: "arg", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    /* [noscript] attribute Noncompat noncompatProp; */
                    Method {
                        name: "GetNoncompatProp",
                        params: &[Param { name: "aNoncompatProp", ty: "*mut Noncompat" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetNoncompatProp",
                        params: &[Param { name: "aNoncompatProp", ty: "Noncompat" }],
                        ret: "::nserror::nsresult",
                    },

                    /* [noscript] void noncompatMethod (in Noncompat arg); */
                    Method {
                        name: "NoncompatMethod",
                        params: &[Param { name: "arg", ty: "Noncompat" }],
                        ret: "::nserror::nsresult",
                    },

                    /* [noscript] attribute long noscriptProp; */
                    Method {
                        name: "GetNoscriptProp",
                        params: &[Param { name: "aNoscriptProp", ty: "*mut i32" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetNoscriptProp",
                        params: &[Param { name: "aNoscriptProp", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    /* [noscript] void noscriptMethod (in long arg); */
                    Method {
                        name: "NoscriptMethod",
                        params: &[Param { name: "arg", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        ]; D}

