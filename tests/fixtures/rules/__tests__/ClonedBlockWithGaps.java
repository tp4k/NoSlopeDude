package fixtures.rules;

public class ClonedBlockWithGaps {

    void first() {
        System.out.println("a");
        System.out.println("b");
    }

    void second() {
        System.out.println("a");
        // duplicated body, comment line

        System.out.println("b");
    }
}
