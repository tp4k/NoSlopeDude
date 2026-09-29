function m(b: boolean): number {
    let x = 0;
    try {
        x = 1;
    } catch (e) {
        return -1;
    }
    return x;
}
