class StructuralPredicates {
    int m(boolean b) {
        int x = 0;
        try {
            x = 1;
        } catch (Exception e) {
            return -1;
        }
        return x;
    }
}
