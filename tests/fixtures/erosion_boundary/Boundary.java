// B4: pins one callable exactly at cc == CC_EROSION_THRESHOLD (10). Nine
// independent `if` statements (no `else`, so `JAVA-REDUNDANT-ELSE-AFTER-
// RETURN` never fires) -- each its own D7 `Branch` decision point, D9's
// base 1 plus 9 gives cc == 10 -- and nothing else, so `cc > 10` (not
// eroded) and a mutated `cc >= 10` (eroded) disagree on it.
class Boundary {
    int classify(int a) {
        int result = 0;
        if (a == 0) { result = 1; }
        if (a == 1) { result = 2; }
        if (a == 2) { result = 3; }
        if (a == 3) { result = 4; }
        if (a == 4) { result = 5; }
        if (a == 5) { result = 6; }
        if (a == 6) { result = 7; }
        if (a == 7) { result = 8; }
        if (a == 8) { result = 9; }
        return result;
    }
}
