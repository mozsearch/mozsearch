import java.io.IOException;

public class ExceptionTest {
  public static void main(String[] args) {
    try {
      throw new IOException("test");
    } catch (final IOException exception) {
    }
  }
}
