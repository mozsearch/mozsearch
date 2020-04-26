# Generate JSON cross reference files for mozsearch

## Usage
```
java -jar mozsearch-java-all.jar <source code path> <output JSON path>
```

## Build with bazel
```
bazel build //:JavaAnalyze
```

## Run test with bazel
```
bazel test //:IndexingTest
```

## Additional Information
If `ANDROID_SDK_ROOT` is set, we use android's jar file to resolve symbols (but it spends a lot of memory).
