java_binary(
    name = "JavaAnalyze",
    srcs = glob(["src/main/java/org/mozilla/mozsearch/*.java"]),
    main_class = "org.mozilla.mozsearch.JavaAnalyze",
    deps = [
        "@maven//:com_github_javaparser_javaparser_core",
        "@maven//:com_github_javaparser_javaparser_symbol_solver_core",
        "@maven//:org_json_json",
    ],
)

java_test(
    name = "IndexingTest",
    srcs = glob(["src/test/java/org/mozilla/mozsearch/*.java"]),
    test_class = "org.mozilla.mozsearch.IndexingTest",
    data = glob([
        "src/test/resources/data/*.java",
        "src/test/resources/result/*.out",
    ]),
    deps = [
        ":JavaAnalyze",
        "@maven//:com_github_javaparser_javaparser_core",
        "@maven//:com_github_javaparser_javaparser_symbol_solver_core",
        "@maven//:org_json_json",
        "@maven//:junit_junit",
    ],
)
