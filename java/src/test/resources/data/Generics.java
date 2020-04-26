import java.util.ArrayList;
import java.util.List;

public class Generics {
  public static void main(String[] args) {
    List<String> list = new ArrayList<String>();
    list.add("foo");

    for (String item : list) {
      System.out.println(item);
    }
  }
}
