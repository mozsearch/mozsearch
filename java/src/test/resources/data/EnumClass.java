public class EnumClass {
  enum PRIORITY {
    LOW,
    MEDIUM,
    HIGH
  };

  private PRIORITY getPriority() {
    return PRIORITY.MEDIUM;
  }

  public static void main(String[] args) {}
}
