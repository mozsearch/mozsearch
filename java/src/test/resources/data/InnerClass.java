public class InnerClass {
  public int x = 0;

  class First {
    public int x = 1;
  }

  public static void main(String[] args) {
    InnerClass first = new InnerClass();
    InnerClass.First v = first.new First();
    v.x = 2;
  }
}
