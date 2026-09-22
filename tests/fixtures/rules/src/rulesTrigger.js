function unreachableAfterReturn() {
    console.log("a");
    return;
    console.log("b");
    console.log("c");
}

function emptyCatch() {
    try {
        risky();
    } catch (e) {
    }
}

function redundantElseAfterReturn(x) {
    if (x > 0) {
        return 1;
    } else {
        return 0;
    }
}

function risky() {}
