function lowComplexity(a, b) {
    if (a > 0) { a = 1; }
    for (let i = 0; i < a; i++) { a++; }
    while (a > 0) { a--; }
    for (const x of [1, 2, 3]) { a += x; }
    let t = a > 0 ? 1 : 2;
    let c = a > 0 && b > 0;
    switch (a) { case 1: break; }
    let done = true;
    return a;
}
