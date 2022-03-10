//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_utils.idl
//


{static D: &'static [Interface] = &[

        Interface {
            name: "nsIXPCTestFunctionInterface",
            base: Some("nsISupports"),
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
            methods: Ok(&[
                    /* nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f); */
                    Method {
                        name: "DoubleWrapFunction",
                        params: &[Param { name: "f", ty: "*const nsIXPCTestFunctionInterface" }, Param { name: "_retval", ty: "*mut *const nsIXPCTestFunctionInterface" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        ]; D}

