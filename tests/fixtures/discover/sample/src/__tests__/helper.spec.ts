import { greet } from "../api/svc";

test("greets", () => {
    expect(greet("world")).toBe("hello, world");
});
