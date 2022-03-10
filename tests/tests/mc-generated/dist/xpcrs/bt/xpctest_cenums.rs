//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_cenums.idl
//


{static D: &'static [Interface] = &[

        Interface {
            name: "nsIXPCTestCEnums",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* void testCEnumInput (in nsIXPCTestCEnums_testFlagsExplicit abc); */
                    Method {
                        name: "TestCEnumInput",
                        params: &[Param { name: "abc", ty: " u8" }],
                        ret: "::nserror::nsresult",
                    },

                    /* nsIXPCTestCEnums_testFlagsExplicit testCEnumOutput (); */
                    Method {
                        name: "TestCEnumOutput",
                        params: &[Param { name: "_retval", ty: "*mut u8" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        ]; D}

