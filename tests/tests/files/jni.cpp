struct JNIEnv {};
struct jobject {};
struct jclass {};

static void Java_sample_Jni_autoNativeStaticMethod(JNIEnv *, jclass)
{
    // Real code
}

static void Java_sample_Jni_autoNativeMethod(JNIEnv *, jobject)
{
    // Real code
}

class __attribute__((annotate("binding_to", "jvm", "class", "S_jvm_sample/Jni#"))) Jni
{
    __attribute__((annotate("binding_to", "jvm", "method", "S_jvm_sample/Jni#javaStaticMethod().")))
    static void javaStaticMethod()
    {
        // Wrapper
    }
    __attribute__((annotate("binding_to", "jvm", "method", "S_jvm_sample/Jni#javaMethod().")))
    void javaMethod()
    {
        // Wrapper
    }

    __attribute__((annotate("binding_to", "jvm", "getter", "S_jvm_sample/Jni#javaField.")))
    int javaField()
    {
        // Wrapper
        return 0;
    }
    __attribute__((annotate("binding_to", "jvm", "setter", "S_jvm_sample/Jni#javaField.")))
    void javaField(int)
    {
        // Wrapper
    }
    __attribute__((annotate("binding_to", "jvm", "const", "S_jvm_sample/Jni#javaConst.")))
    static constexpr int javaConst = 5;

    void user() {
        javaStaticMethod();
        javaMethod();
        javaField();
        javaField(javaConst);
    }
};

class __attribute__((annotate("bound_as", "jvm", "class", "S_jvm_sample/Jni#"))) Nji
{
    __attribute__((annotate("bound_as", "jvm", "method", "S_jvm_sample/Jni#nativeStaticMethod().")))
    static void nativeStaticMethod()
    {
        // Real code
    }
    __attribute__((annotate("bound_as", "jvm", "method", "S_jvm_sample/Jni#nativeMethod().")))
    void nativeMethod()
    {
        // Real code
    }

    void user() {
        nativeStaticMethod();
        nativeMethod();
    }
};
