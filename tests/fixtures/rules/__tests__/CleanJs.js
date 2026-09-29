function classify(x) {
    if (x > 0) {
        return 1;
    }
    return 0;
}

function safeCatch() {
    try {
        risky();
    } catch (e) {
        console.warn("failed");
    }
}

function ignoredCatch() {
    try {
        risky();
    } catch (e) {
        // explicitly ignored: best-effort cleanup
    }
}

function compute(x) {
    if (x > 0) {
        console.log("positive");
    } else {
        return -1;
    }
    return 0;
}

function risky() {}

function withHoistedFunction() {
    return 1;

    function helper() {
        return 2;
    }
}

function withHoistedGenerator() {
    return 1;

    function* helperGenerator() {
        yield 2;
    }
}
