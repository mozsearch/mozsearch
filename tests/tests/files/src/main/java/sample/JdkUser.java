package sample;

import java.util.Map;

/**
 * Uses the JDK's classes, whose documentation their external symbols link to
 * (see tools/src/jvm_docs.rs).
 */
public class JdkUser {
    public int largest() {
        return Integer.MAX_VALUE;
    }

    public String describe(Map.Entry<String, String> entry) {
        return String.valueOf(entry.getKey());
    }
}
