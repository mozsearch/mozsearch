use std::collections::HashMap;
use std::path::Path;

pub struct LanguageSpec {
    pub reserved_words: HashMap<String, String>,
    pub hash_comment: bool,
    pub c_style_comments: bool,
    pub backtick_strings: bool,
    pub regexp_literals: bool,
    pub triple_quote_literals: bool,
    pub c_preprocessor: bool,
}

fn make_reserved(v: &Vec<&str>) -> HashMap<String, String> {
    let mut reserved_words = HashMap::new();
    for word in v {
        reserved_words.insert(word.to_string(), "style=\"color: blue;\" ".to_string());
    }
    reserved_words
}

lazy_static! {
    static ref RESERVED_WORDS_JS : Vec<&'static str> = vec![
        "abstract", "else", "instanceof", "super",
        "boolean", "enum", "int", "switch",
        "break", "export", "interface", "synchronized",
        "byte", "extends", "let", "this",
        "case", "false", "long", "throw",
        "catch", "final", "native", "throws",
        "char", "finally", "new", "transient",
        "class", "float", "null", "true",
        "const", "for", "package", "try",
        "continue", "function", "private", "typeof",
        "debugger", "goto", "protected", "var",
        "default", "if", "public", "void",
        "delete", "implements", "return", "volatile",
        "do", "import", "short", "while",
        "double", "in", "static", "with",
        "get", "set",
    ];

    static ref RESERVED_WORDS_CPP : Vec<&'static str> = vec![
        "alignas", "alignof", "and", "and_eq", "asm", "atomic_cancel",
        "atomic_commit", "atomic_noexcept", "auto", "bitand", "bitor", "bool", "break",
        "case", "catch", "char", "char16_t", "char32_t", "class", "compl", "concept",
        "const", "constexpr", "const_cast", "continue", "decltype", "default", "delete",
        "do", "double", "dynamic_cast", "else", "enum", "explicit", "export", "extern", "false",
        "float", "for", "friend", "goto", "if", "inline", "int", "import", "long", "module",
        "mutable", "namespace", "new", "noexcept", "not", "not_eq", "nullptr", "operator",
        "or", "or_eq", "private", "protected", "public", "register", "reinterpret_cast",
        "requires", "return", "short", "signed", "sizeof", "static", "static_assert",
        "static_cast", "struct", "switch", "synchronized", "template", "this", "thread_local",
        "throw", "true", "try", "typedef", "typeid", "typename", "union", "unsigned",
        "using", "virtual", "void", "volatile", "wchar_t", "while", "xor", "xor_eq",
        "#if", "#ifdef", "#ifndef", "#else", "#elif", "#endif", "#define", "#undef",
        "#include", "#error", "defined",
    ];

    static ref RESERVED_WORDS_IPDL : Vec<&'static str> = vec![
        "answer", "as", "async", "both", "bridges", "call", "child", "class",
        "compress", "compressall", "__delete__", "delete", "from", "goto", "high",
        "include", "intr", "manager", "manages", "namespace", "normal", "nullable",
        "opens", "or", "parent", "prio", "protocol", "recv", "returns", "send",
        "spawns", "start", "state", "struct", "sync", "union", "upto", "urgent",
        "using",
    ];

    static ref RESERVED_WORDS_IDL : Vec<&'static str> = vec![
        "const", "interface", "in", "inout", "out", "attribute", "raises",
        "readonly", "native", "typedef",
        "array", "shared", "iid_is", "size_is", "retval",
        "boolean", "void", "octet", "short", "long", "long",
        "unsigned", "float", "double", "char", "string", "wchar", "wstring",
        "nsid", "domstring", "utf8string", "cstring", "astring", "jsval",
        "uuid", "scriptable", "builtinclass", "function", "noscript", "deprecated",
        "object", "main_process_scriptable_only",
    ];

    static ref RESERVED_WORDS_WEBIDL : Vec<&'static str> = vec![
        "module", "interface", "partial", "dictionary", "exception", "enum", "callback",
        "typedef", "implements", "const", "null", "true", "false", "serializer",
        "stringifier", "jsonifier", "unrestricted", "attribute", "readonly", "inherit",
        "static", "getter", "setter", "creator", "deleter", "legacycaller", "optional",
        "Date", "DOMString", "ByteString", "USVString", "any", "boolean", "byte",
        "double", "float", "long", "object", "octet", "Promise", "required", "sequence",
        "MozMap", "short", "unsigned", "void", "ArrayBuffer", "SharedArrayBuffer", "or",
        "maplike", "setlike", "iterable",
        "Exposed", "ChromeOnly", "ChromeConstructor", "Pref", "Func", "AvailableIn",
        "CheckAnyPermissions", "CheckAllPermissions", "JSImplementation", "HeaderFile",
        "NavigatorProperty", "AvailableIn", "Func", "CheckAnyPermissions", "CheckAllPermissions",
        "Deprecated", "NeedResolve", "OverrideBuiltins", "ChromeOnly", "Unforgeable",
        "UnsafeInPrerendering", "LegacyEventInit", "ProbablyShortLivingObject", "ArrayClass",
        "Clamp", "Constructor", "EnforceRange", "ExceptionClass", "Exposed", "ImplicitThis",
        "Global", "PrimaryGlobal", "LegacyArrayClass", "LegacyUnenumerableNamedProperties",
        "LenientSetter", "LenientThis", "NamedConstructor", "NewObject", "NoInterfaceObject",
        "OverrideBuiltins", "PutForwards", "Replaceable", "SameObject", "SecureContext",
        "Throws", "TreatNonObjectAsNull", "TreatNullAs", "Unforgeable", "Unscopable",
    ];

    static ref RESERVED_WORDS_PYTHON : Vec<&'static str> = vec![
        "and", "del", "from", "not", "while",
        "as", "elif", "global", "or", "with",
        "assert", "else", "if", "pass", "yield",
        "break", "except", "import", "print",
        "class", "exec", "in", "raise", "continue",
        "finally", "is", "return",
        "def", "for", "lambda", "try",
    ];

    static ref JS_SPEC : LanguageSpec = LanguageSpec {
        reserved_words: make_reserved(&*RESERVED_WORDS_JS),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: true,
        regexp_literals: true,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    static ref CPP_SPEC : LanguageSpec = LanguageSpec {
        reserved_words: make_reserved(&*RESERVED_WORDS_CPP),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: true,
    };

    static ref IPDL_SPEC : LanguageSpec = LanguageSpec {
        reserved_words: make_reserved(&*RESERVED_WORDS_IPDL),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    static ref IDL_SPEC : LanguageSpec = LanguageSpec {
        reserved_words: make_reserved(&*RESERVED_WORDS_IDL),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    static ref WEBIDL_SPEC : LanguageSpec = LanguageSpec {
        reserved_words: make_reserved(&*RESERVED_WORDS_WEBIDL),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    static ref PYTHON_SPEC : LanguageSpec = LanguageSpec {
        reserved_words: make_reserved(&*RESERVED_WORDS_PYTHON),
        hash_comment: true,
        c_style_comments: false,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: true,
        c_preprocessor: false,
    };
}

pub enum FormatAs {
    FormatCLike(&'static LanguageSpec),
    FormatTagLike(&'static LanguageSpec),
    Plain,
    Binary,
}

pub fn select_formatting(filename: &str) -> FormatAs {
    let ext = match Path::new(filename).extension() {
        Some(ext) => ext.to_str().unwrap(),
        None => "",
    };
    match ext {
        "c" | "cc" | "cpp" | "h" | "hh" => FormatAs::FormatCLike(&*CPP_SPEC),
        "ipdl" | "ipdlh" => FormatAs::FormatCLike(&*IPDL_SPEC),
        "idl" => FormatAs::FormatCLike(&*IDL_SPEC),
        "webidl" => FormatAs::FormatCLike(&*WEBIDL_SPEC),
        "js" | "jsm" | "json" => FormatAs::FormatCLike(&*JS_SPEC),
        "py" | "build" => FormatAs::FormatCLike(&*PYTHON_SPEC),

        "html" | "htm" | "xml" | "xul" => FormatAs::FormatTagLike(&*JS_SPEC),

        "ogg" | "ttf" | "xpi" | "png" | "bcmap" |
        "gif" | "ogv" | "jpg" | "bmp" | "icns" | "ico" |
        "mp4" | "sqlite" | "jar" | "webm" | "woff" |
        "class" | "m4s" | "mgif" | "wav" | "opus" |
        "mp3" | "otf" => FormatAs::Binary,

        _ => FormatAs::Plain,
    }
}
