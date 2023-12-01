package sample;

public class Jni {
    public static native void autoNativeStaticMethod();
    public native void autoNativeMethod();

    public static native void nativeStaticMethod();
    public native void nativeMethod();

    public static void javaStaticMethod() {}
    public void javaMethod() {}
    public int javaField;
    public static final int javaConst = 5;

    private void user() {
        nativeStaticMethod();
        nativeMethod();

        javaStaticMethod();
        javaMethod();
        javaField = javaConst;
    }
}
