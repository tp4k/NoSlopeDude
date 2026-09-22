interface Shape {
    int area();
}

abstract class AbstractShape implements Shape {
    abstract int perimeter();
}

record Point(int x, int y) {
    Point {
        if (x < 0) {
            x = 0;
        }
    }
}

class CallableKinds {
    static {
        int a = 1;
    }

    CallableKinds() {
        int a = 1;
    }

    int compute(int a) {
        Runnable r = () -> {
            int b = 1;
        };
        return a;
    }
}
