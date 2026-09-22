class HighComplexity {
    int compute(int a, int b) {
        if (a > 0 && b > 0) { a = 1; }
        for (int i = 0; i < a; i++) { a++; }
        while (a > 0) { a--; }
        do { a++; } while (a < 10);
        try { a = 1; } catch (RuntimeException e) { a = 2; }
        int t = a > 0 ? 1 : 2;
        switch (a) { case 1: break; }
        switch (a) { case 2: break; case 3: break; }
        boolean c = a > 0 || b > 0;
    }
}
