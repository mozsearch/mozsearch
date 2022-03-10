//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_attributes.idl
//


{static D: &'static [Interface] = &[

        Interface {
            name: "nsIXPCTestObjectReadOnly",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* readonly attribute string strReadOnly; */
                    Method {
                        name: "GetStrReadOnly",
                        params: &[Param { name: "aStrReadOnly", ty: "*mut *const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    /* readonly attribute boolean boolReadOnly; */
                    Method {
                        name: "GetBoolReadOnly",
                        params: &[Param { name: "aBoolReadOnly", ty: "*mut bool" }],
                        ret: "::nserror::nsresult",
                    },

                    /* readonly attribute short shortReadOnly; */
                    Method {
                        name: "GetShortReadOnly",
                        params: &[Param { name: "aShortReadOnly", ty: "*mut i16" }],
                        ret: "::nserror::nsresult",
                    },

                    /* readonly attribute long longReadOnly; */
                    Method {
                        name: "GetLongReadOnly",
                        params: &[Param { name: "aLongReadOnly", ty: "*mut i32" }],
                        ret: "::nserror::nsresult",
                    },

                    /* readonly attribute float floatReadOnly; */
                    Method {
                        name: "GetFloatReadOnly",
                        params: &[Param { name: "aFloatReadOnly", ty: "*mut libc::c_float" }],
                        ret: "::nserror::nsresult",
                    },

                    /* readonly attribute char charReadOnly; */
                    Method {
                        name: "GetCharReadOnly",
                        params: &[Param { name: "aCharReadOnly", ty: "*mut libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    /* readonly attribute PRTime timeReadOnly; */
                    Method {
                        name: "GetTimeReadOnly",
                        params: &[Param { name: "aTimeReadOnly", ty: "*mut PRTime" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        Interface {
            name: "nsIXPCTestObjectReadWrite",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* attribute string stringProperty; */
                    Method {
                        name: "GetStringProperty",
                        params: &[Param { name: "aStringProperty", ty: "*mut *const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetStringProperty",
                        params: &[Param { name: "aStringProperty", ty: "*const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    /* attribute boolean booleanProperty; */
                    Method {
                        name: "GetBooleanProperty",
                        params: &[Param { name: "aBooleanProperty", ty: "*mut bool" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetBooleanProperty",
                        params: &[Param { name: "aBooleanProperty", ty: "bool" }],
                        ret: "::nserror::nsresult",
                    },

                    /* attribute short shortProperty; */
                    Method {
                        name: "GetShortProperty",
                        params: &[Param { name: "aShortProperty", ty: "*mut i16" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetShortProperty",
                        params: &[Param { name: "aShortProperty", ty: "i16" }],
                        ret: "::nserror::nsresult",
                    },

                    /* attribute long longProperty; */
                    Method {
                        name: "GetLongProperty",
                        params: &[Param { name: "aLongProperty", ty: "*mut i32" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetLongProperty",
                        params: &[Param { name: "aLongProperty", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    /* attribute float floatProperty; */
                    Method {
                        name: "GetFloatProperty",
                        params: &[Param { name: "aFloatProperty", ty: "*mut libc::c_float" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetFloatProperty",
                        params: &[Param { name: "aFloatProperty", ty: "libc::c_float" }],
                        ret: "::nserror::nsresult",
                    },

                    /* attribute char charProperty; */
                    Method {
                        name: "GetCharProperty",
                        params: &[Param { name: "aCharProperty", ty: "*mut libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetCharProperty",
                        params: &[Param { name: "aCharProperty", ty: "libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    /* attribute PRTime timeProperty; */
                    Method {
                        name: "GetTimeProperty",
                        params: &[Param { name: "aTimeProperty", ty: "*mut PRTime" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetTimeProperty",
                        params: &[Param { name: "aTimeProperty", ty: "PRTime" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        ]; D}

