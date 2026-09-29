class Decisions {
    int m(int x, boolean b) {
        if (x > 0) {
            x = 1;
        }
        for (int i = 0; i < x; i++) {
            x = x - 1;
        }
        while (x > 0) {
            x = x - 1;
        }
        switch (x) {
            case 1:
                x = 2;
                break;
            default:
                x = 3;
        }
        try {
            x = 4;
        } catch (Exception e) {
            x = 5;
        }
        int y = b ? 1 : 2;
        boolean c = b && b;
        boolean d = b || b;
        return y;
    }
}
