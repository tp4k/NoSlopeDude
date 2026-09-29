function forOfOptional(items, maybe) {
    let total = 0;
    for (const item of items) {
        total += item;
    }
    const value = maybe?.value;
    const fallback = maybe ?? 0;
    return total + fallback + value;
}
