package fixtures.rules;

public class JavaRulesFixture {

    void unreachableAfterReturn() {
        System.out.println("a");
        return;
        System.out.println("b");
        System.out.println("c");
    }

    void emptyCatch() {
        try {
            risky();
        } catch (Exception e) {
        }
    }

    int redundantElseAfterReturn(int x) {
        if (x > 0) {
            return 1;
        } else {
            return 0;
        }
    }

    void risky() {
    }
}
