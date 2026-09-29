class Decisions {
    int decide(int a, int b) {
        if (a > 0) {
            a = 1;
        } else if (b > 0) {
            a = 2;
        } else {
            a = 3;
        }
        for (int i = 0; i < a; i++) {
            a++;
        }
        for (int x : new int[] {1, 2, 3}) {
            a += x;
        }
        while (a > 0) {
            a--;
        }
        do {
            a++;
        } while (a < 10);
        try {
            a = 1;
        } catch (RuntimeException e) {
            a = 2;
        } catch (Exception e) {
            a = 3;
        } finally {
            a = 4;
        }
        int t = a > 0 ? 1 : 2;
        switch (a) {
            case 1:
                break;
            case 2:
                break;
            case 3:
                break;
            default:
                break;
        }
        boolean c = a > 0 && b > 0 || a < 0;
        return t;
    }
}
