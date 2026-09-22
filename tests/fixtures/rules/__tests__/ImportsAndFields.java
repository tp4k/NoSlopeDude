package fixtures.rules;

import java.util.List;

public class ImportsAndFields {

    private int counter;
    private String name;

    int increment() {
        counter = counter + 1;
        return counter;
    }
}
