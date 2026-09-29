function outer(a) {
    const inner = (b) => (b > 0 ? b : -b);
    if (a > 0) {
        return inner(a);
    }
    return 0;
}
