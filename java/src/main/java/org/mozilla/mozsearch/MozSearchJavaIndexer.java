package org.mozilla.mozsearch;

import com.github.javaparser.ParseProblemException;
import com.github.javaparser.StaticJavaParser;
import com.github.javaparser.ast.CompilationUnit;
import com.github.javaparser.symbolsolver.JavaSymbolSolver;
import com.github.javaparser.symbolsolver.resolution.typesolvers.CombinedTypeSolver;
import com.github.javaparser.symbolsolver.resolution.typesolvers.JarTypeSolver;
import com.github.javaparser.symbolsolver.resolution.typesolvers.JavaParserTypeSolver;
import com.github.javaparser.symbolsolver.resolution.typesolvers.ReflectionTypeSolver;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;

public class MozSearchJavaIndexer {
  private Path mSourceDir;
  private Path mOutputDir;
  private int mTimeout = -1;
  private int mThreadPoolCount = 4;

  public MozSearchJavaIndexer(final Path sourceDir, final Path outputDir) {
    mSourceDir = sourceDir.toAbsolutePath();
    mOutputDir = outputDir.toAbsolutePath();
  }

  public void setTimeout(int timeout) {
    mTimeout = timeout;
  }

  public void outputIndexes() {
    try {
      indexAllChildren(mSourceDir, mSourceDir, mOutputDir);
    } catch (IOException exception) {
      System.err.println(exception);
    }
  }

  private void indexAllChildren(final Path currentDir, final Path srcDir, final Path outputDir)
      throws IOException {
    if (currentDir.toFile().getName().equals(".git")
        || currentDir.toFile().getName().equals(".hg")) {
      return;
    }
    ArrayList<Path> javaFiles = new ArrayList<Path>();
    List<Path> files = Files.list(currentDir).collect(Collectors.toList());
    for (Path file : files) {
      if (Files.isDirectory(file) && !Files.isSymbolicLink(file)) {
        indexAllChildren(file, srcDir, outputDir);
      } else if (file.toString().endsWith(".java")) {
        javaFiles.add(file);
      }
    }
    makeIndexes(javaFiles, srcDir, outputDir);
  }

  private Path getRootPath(final Path file, final String packageName) {
    String path = packageName;
    Path root = file.getParent();
    String leafName = root.getFileName().toString();
    root = root.getParent();

    // Find root directory of this Java source package.
    // If directory structure isn't Java package structure, we don't return
    // root directory.
    while (path.contains(".")) {
      if (!leafName.equals(path.substring(path.lastIndexOf(".") + 1))) {
        return null;
      }

      leafName = root.getFileName().toString();
      root = root.getParent();
      path = path.substring(0, path.lastIndexOf("."));
    }

    if (!root.startsWith(mSourceDir)) {
      return null;
    }
    return root;
  }

  private void makeIndexes(final List<Path> files, final Path srcDir, final Path outputDir) {
    if (files.isEmpty()) {
      return;
    }

    final CombinedTypeSolver solver = new CombinedTypeSolver();
    solver.add(new ReflectionTypeSolver());

    // This is cached dir list not to add duplicated entry
    final ArrayList<Path> dirs = new ArrayList<Path>();

    // Add root directory from package syntax
    StaticJavaParser.getConfiguration().setSymbolResolver(null);
    for (Path file : files) {
      try {
        final CompilationUnit unit = StaticJavaParser.parse(file);
        if (unit.getPackageDeclaration().isPresent()) {
          final String packageName = unit.getPackageDeclaration().get().getName().toString();
          final Path rootDir = getRootPath(file, packageName);
          if (rootDir != null && !dirs.contains(rootDir)) {
            solver.add(new JavaParserTypeSolver(rootDir));
            dirs.add(rootDir);
          }
        }
      } catch (Exception exception) {
        exception.printStackTrace();
      }

      if (!dirs.contains(file.getParent())) {
        solver.add(new JavaParserTypeSolver(file.getParent()));
        dirs.add(file.getParent());
      }
    }

    // Set Android SDK's JAR using ANDROID_SDK_ROOT
    final String sdkroot = System.getenv("ANDROID_SDK_ROOT");
    if (sdkroot != null && sdkroot.length() > 0) {
      try {
        final String[] apis = new String[] {"android-31", "android-30", "android-29", "android-28"};
        for (String api : apis) {
          final Path sdkrootPath = Paths.get(sdkroot, "platforms", api, "android.jar");
          if (Files.exists(sdkrootPath)) {
            solver.add(new JarTypeSolver(sdkrootPath));
            break;
          }
        }
      } catch (IOException exception) {
      }
    }

    StaticJavaParser.getConfiguration().setSymbolResolver(new JavaSymbolSolver(solver));

    for (Path file : files) {
      final Path output =
          Paths.get(
              outputDir.toString(), file.toString().substring(srcDir.toString().length() + 1));
      try {
        makeIndex(file, output);
      } catch (Exception exception) {
        exception.printStackTrace();
        try {
          Files.delete(output);
        } catch (IOException ioexception) {
        }
      }
    }
  }

  private void makeIndex(final Path file, final Path outputPath)
      throws IOException, ParseProblemException {
    if (!file.toString().endsWith(".java")) {
      return;
    }

    System.out.println("Processing " + file.toString() + " ");

    final CompilationUnit unit = StaticJavaParser.parse(file);
    final MozSearchJSONOutputVisitor visitor = new MozSearchJSONOutputVisitor(outputPath);
    if (mTimeout > 0) {
      visitor.setTimeout(mTimeout);
    }
    unit.accept(visitor, null);
  }
}
