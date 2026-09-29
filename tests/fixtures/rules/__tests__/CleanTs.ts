function withInterfaceAfterReturn(): number {
    return 1;

    interface Unused {
        value: number;
    }
}

function withTypeAliasAfterReturn(): number {
    return 1;

    type UnusedAlias = number;
}
