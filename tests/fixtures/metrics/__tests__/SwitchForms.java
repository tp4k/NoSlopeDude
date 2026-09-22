class SwitchForms {
    int colonForm(int a) {
        switch (a) {
            case 1:
                break;
            case 2:
                break;
            default:
                break;
        }
        return a;
    }

    int arrowForm(int a) {
        switch (a) {
            case 1 -> a = 1;
            case 2 -> a = 2;
            default -> a = 3;
        }
        return a;
    }
}
