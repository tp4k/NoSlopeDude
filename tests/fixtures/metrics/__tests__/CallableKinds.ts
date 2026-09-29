function regular(a: number): number {
    return a;
}

function* genFn(a: number) {
    yield a;
}

function signatureOnly(a: number): void;

const fnExpr = function (a: number): number {
    return a;
};

const arrow = (a: number): number => a;

class Widget {
    method(a: number): number {
        return a;
    }
}

abstract class AbstractWidget {
    abstract render(): void;
}

interface Renderer {
    render(): void;
}
