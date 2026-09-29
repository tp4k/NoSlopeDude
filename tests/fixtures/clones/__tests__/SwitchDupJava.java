public class SwitchDupJava {
    void run(int x) {
        switch (x) {
            case 1:
                alpha(one,
                      two,
                      three);
                beta(four,
                     five,
                     six);
                gamma(seven,
                      eight,
                      nine);
                break;
            case 2:
                alpha(one,
                      two,
                      three);
                beta(four,
                     five,
                     six);
                gamma(seven,
                      eight,
                      nine);
                break;
            default:
                otherwise();
        }
    }
}
