// B4: pins one callable exactly at cc == CC_EROSION_THRESHOLD (10). The
// chain below is nine `if`/`else if` branches -- each its own D7 `Branch`
// decision point, D9's base 1 plus 9 gives cc == 10 -- and nothing else, so
// `cc > 10` (not eroded) and a mutated `cc >= 10` (eroded) disagree on it.
class Boundary {
    int classify(int a) {
        if (a == 0) {
            return 0;
        } else if (a == 1) {
            return 1;
        } else if (a == 2) {
            return 2;
        } else if (a == 3) {
            return 3;
        } else if (a == 4) {
            return 4;
        } else if (a == 5) {
            return 5;
        } else if (a == 6) {
            return 6;
        } else if (a == 7) {
            return 7;
        } else if (a == 8) {
            return 8;
        }
        return -1;
    }
}
