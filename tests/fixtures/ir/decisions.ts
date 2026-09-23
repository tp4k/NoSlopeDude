function m(x: number, b: boolean): number {
    if (x > 0) {
        x = 1;
    }
    for (let i = 0; i < x; i++) {
        x = x - 1;
    }
    while (x > 0) {
        x = x - 1;
    }
    switch (x) {
        case 1:
            x = 2;
            break;
        default:
            x = 3;
    }
    try {
        x = 4;
    } catch (e) {
        x = 5;
    }
    let y = b ? 1 : 2;
    let c = b && b;
    let d = b || b;
    return y;
}
