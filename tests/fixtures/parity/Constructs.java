class Constructs {
    void ifConstruct(int a) {
        if (a > 0) {
            a = 1;
        } else {
            a = 2;
        }
    }

    void forConstruct(int a) {
        for (int i = 0; i < a; i++) {
            a++;
        }
    }

    void whileConstruct(int a) {
        while (a > 0) {
            a--;
        }
    }

    void switchConstruct(int a) {
        switch (a) {
            case 1:
                break;
            default:
                break;
        }
    }

    void catchConstruct() {
        try {
            int a = 1;
        } catch (Exception e) {
            int b = 2;
        } finally {
            int c = 3;
        }
    }

    void ternaryConstruct(int a) {
        int t = a > 0 ? 1 : 2;
    }

    void andConstruct(boolean a, boolean b) {
        boolean c = a && b;
    }

    void orConstruct(boolean a, boolean b) {
        boolean c = a || b;
    }
}
