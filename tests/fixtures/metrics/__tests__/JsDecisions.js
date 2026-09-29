function decide(a, b) {
    if (a > 0) {
        a = 1;
    } else {
        a = 2;
    }
    for (let i = 0; i < a; i++) {
        a++;
    }
    for (const x in { a: 1 }) {
        a += 1;
    }
    while (a > 0) {
        a--;
    }
    do {
        a++;
    } while (a < 10);
    try {
        a = 1;
    } catch (e) {
        a = 2;
    } finally {
        a = 3;
    }
    let t = a > 0 ? 1 : 2;
    switch (a) {
        case 1:
            break;
        case 2:
            break;
        default:
            break;
    }
    let c1 = a > 0 && b > 0;
    let c2 = a > 0 || b > 0;
    let c3 = a ?? b;
    return t;
}
