package org.mozilla.mozsearch;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Paths;
import junit.framework.Test;
import junit.framework.TestCase;
import junit.framework.TestSuite;

public class IndexingTest extends TestCase {
  /**
   * Create the test case
   *
   * @param testName name of the test case
   */
  public IndexingTest(String testName) {
    super(testName);
  }

  /** @return the suite of tests being tested */
  public static Test suite() {
    return new TestSuite(IndexingTest.class);
  }

  public void testIndexing() throws IOException {
    MozSearchJavaIndexer indexer =
        new MozSearchJavaIndexer(Paths.get("./src/test/resources/data"), Paths.get("/tmp"));
    indexer.outputIndexes();
    byte[] f1 = Files.readAllBytes(Paths.get("/tmp/HelloWorld.java"));
    byte[] f2 = Files.readAllBytes(Paths.get("./src/test/resources/result/HelloWorld.java.out"));
    assertTrue(f1.length == f2.length);
    Files.delete(Paths.get("/tmp/HelloWorld.java"));

    f1 = Files.readAllBytes(Paths.get("/tmp/InnerClass.java"));
    f2 = Files.readAllBytes(Paths.get("./src/test/resources/result/InnerClass.java.out"));
    assertTrue(f1.length == f2.length);
    Files.delete(Paths.get("/tmp/InnerClass.java"));

    f1 = Files.readAllBytes(Paths.get("/tmp/Generics.java"));
    f2 = Files.readAllBytes(Paths.get("./src/test/resources/result/Generics.java.out"));
    assertTrue(f1.length == f2.length);
    Files.delete(Paths.get("/tmp/Generics.java"));

    f1 = Files.readAllBytes(Paths.get("/tmp/EnumClass.java"));
    f2 = Files.readAllBytes(Paths.get("./src/test/resources/result/EnumClass.java.out"));
    assertTrue(f1.length == f2.length);
    Files.delete(Paths.get("/tmp/EnumClass.java"));

    f1 = Files.readAllBytes(Paths.get("/tmp/ExceptionTest.java"));
    f2 = Files.readAllBytes(Paths.get("./src/test/resources/result/ExceptionTest.java.out"));
    assertTrue(f1.length == f2.length);
    Files.delete(Paths.get("/tmp/ExceptionTest.java"));
  }
}
