package fixtures.rules.broken;

public class Good {

    int classify(int x) {
        if (x > 0) {
            return 1;
        }
        return 0;
    }
}
