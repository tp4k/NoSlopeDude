public class PartialRun {
    void methodA() {
        setupOne();
        alpha(one,
              two,
              three,
              threeB);
        beta(four,
             five,
             six,
             sixB);
        gamma(seven,
              eight,
              nine,
              nineB);
        teardownOne();
    }

    void methodB() {
        differentSetup();
        alpha(one,
              two,
              three,
              threeB);
        beta(four,
             five,
             six,
             sixB);
        gamma(seven,
              eight,
              nine,
              nineB);
        somethingElseEntirely();
    }
}
