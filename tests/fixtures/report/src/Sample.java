public class Sample {
    public int classify(int value) {
        if (value > 0) {
            return 1;
        } else {
            return -1;
        }
    }

    public void risky() {
        try {
            doSomething();
        } catch (Exception e) {
        }
    }

    private void doSomething() {
        System.out.println("working");
    }
}
