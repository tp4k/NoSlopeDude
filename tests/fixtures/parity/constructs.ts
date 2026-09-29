function ifConstruct(a: number): void {
    if (a > 0) {
        a = 1;
    } else {
        a = 2;
    }
}

function forConstruct(a: number): void {
    for (let i = 0; i < a; i++) {
        a++;
    }
}

function whileConstruct(a: number): void {
    while (a > 0) {
        a--;
    }
}

function switchConstruct(a: number): void {
    switch (a) {
        case 1:
            break;
        default:
            break;
    }
}

function catchConstruct(): void {
    try {
        const a = 1;
    } catch (e) {
        const b = 2;
    } finally {
        const c = 3;
    }
}

function ternaryConstruct(a: number): void {
    const t = a > 0 ? 1 : 2;
}

function andConstruct(a: boolean, b: boolean): void {
    const c = a && b;
}

function orConstruct(a: boolean, b: boolean): void {
    const c = a || b;
}
